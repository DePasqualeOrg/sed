// Program entry point and CLI processing
//
// SPDX-License-Identifier: MIT
// Copyright (c) 2025 Diomidis Spinellis
//
// This file is part of the uutils sed package.
// It is licensed under the MIT License.
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.

pub mod command;
pub mod compiler;
pub mod delimited_parser;
pub mod error_handling;
pub mod fast_io;
pub mod fast_regex;
pub mod in_place;
pub mod named_writer;
pub mod processor;
pub mod script_char_provider;
pub mod script_line_provider;

use crate::sed::command::{ProcessingContext, StringSpace};
use crate::sed::compiler::compile;
use crate::sed::processor::process_all_files;
use crate::sed::script_line_provider::ScriptValue;
use clap::{Arg, ArgMatches, Command, arg, crate_version};
use std::collections::HashMap;
use std::path::PathBuf;
use uucore::error::{UResult, UUsageError};
use uucore::format_usage;

const ABOUT: &str = "Stream editor for filtering and transforming text";
const USAGE: &str = "sed [OPTION]... [script] [file]...";

#[uucore::main]
pub fn uumain(args: impl uucore::Args) -> UResult<()> {
    let matches = uu_app().try_get_matches_from(preprocess_in_place_suffix(args))?;

    // Don't use arg_required_else_help when declaring command
    // as it exits with code 2 and we use it to check
    // default matches in tests.
    if !matches.args_present() {
        let _ = uu_app().print_help();
        std::process::exit(1);
    }

    let (scripts, files) = get_scripts_files(&matches)?;
    let mut context = build_context(&matches);

    let executable = compile(scripts, &mut context)?;
    process_all_files(executable, files, &mut context)?;
    Ok(())
}

// Rewrites `-iSUFFIX` (short option with attached suffix, no `=`) into
// `-i=SUFFIX` so the `in-place` Arg still accepts the GNU short attached
// form. The Arg uses `require_equals(true)`, which stops clap from
// greedily consuming the next positional as SUFFIX but also rejects the
// attached short form; the rewrite restores that form. Leaves `-i`,
// `-i=SUFFIX`, `--in-place`, and `--in-place=SUFFIX` alone, and does not
// touch args appearing after a `--` terminator.
//
// Matching GNU sed, `-i` does not participate in short-option stacking:
// `-iE` is parsed as `-i` with SUFFIX `E`, not as `-i -E`. Callers that
// want both `-i` and `-E` must spell them as separate tokens.
//
// Non-UTF-8 args pass through untouched: `matches.get_one::<String>(
// "in-place")` in `build_context` would reject them anyway, so there's
// nothing to be gained by rewriting them at this layer.
fn preprocess_in_place_suffix(args: impl uucore::Args) -> Vec<std::ffi::OsString> {
    use std::ffi::OsString;
    let mut out: Vec<OsString> = Vec::new();
    let mut past_terminator = false;
    for arg in args {
        if past_terminator {
            out.push(arg);
            continue;
        }
        if arg == "--" {
            past_terminator = true;
            out.push(arg);
            continue;
        }
        if let Some(s) = arg.to_str()
            && let Some(suffix) = s.strip_prefix("-i")
            && !suffix.is_empty()
            && !suffix.starts_with('=')
        {
            out.push(OsString::from(format!("-i={suffix}")));
            continue;
        }
        out.push(arg);
    }
    out
}

#[allow(clippy::cognitive_complexity)]
pub fn uu_app() -> Command {
    #[cfg(windows)]
    let util_name = "sed";
    #[cfg(not(windows))]
    let util_name = uucore::util_name();

    Command::new(util_name)
        .version(crate_version!())
        .about(ABOUT)
        .override_usage(format_usage(USAGE))
        .args_override_self(true)
        .infer_long_args(true)
        .args([
            arg!([script] "Script to execute if not otherwise provided."),
            Arg::new("file")
                .help("Input files")
                .value_parser(clap::value_parser!(PathBuf))
                .num_args(0..),
            Arg::new("all-output-files")
                .long("all-output-files")
                .short('a')
                .help("Create or truncate all output files before processing.")
                .action(clap::ArgAction::SetTrue),
            arg!(--debug "Annotate program execution."),
            Arg::new("regexp-extended")
                .short('E')
                .long("regexp-extended")
                .short_alias('r')
                .help("Use extended regular expressions.")
                .action(clap::ArgAction::SetTrue),
            arg!(-e --expression <SCRIPT> "Add script to executed commands.")
                .action(clap::ArgAction::Append),
            // Access with .get_many::<PathBuf>("file")
            Arg::new("script-file")
                .short('f')
                .long("script-file")
                .help("Specify script file.")
                .value_parser(clap::value_parser!(PathBuf))
                .action(clap::ArgAction::Append),
            Arg::new("follow-symlinks")
                .long("follow-symlinks")
                .help("Follow symlinks when processing in place.")
                .action(clap::ArgAction::SetTrue),
            // Access with .get_one::<String>("in-place")
            //
            // `require_equals(true)` prevents clap from greedily consuming
            // the next positional as SUFFIX, which would otherwise turn
            // `sed -i 's/foo/bar/' file` into `-i 's/foo/bar/'` (SUFFIX) +
            // `file` as the script, causing parser errors. The short attached
            // form `-iSUFFIX` (without `=`) is preserved via the argv
            // preprocessor in `uumain`, which rewrites `-iSUFFIX` to
            // `-i=SUFFIX` before clap sees the args.
            Arg::new("in-place")
                .short('i')
                .long("in-place")
                .help("Edit files in place, making a backup if SUFFIX is supplied.")
                .num_args(0..=1)
                .require_equals(true)
                .default_missing_value(""),
            // Access with .get_one::<u32>("line-length")
            arg!(-l --length <NUM> "Specify the 'l' command line-wrap length.")
                .value_parser(clap::value_parser!(u32)),
            arg!(-n --quiet "Suppress automatic printing of pattern space.").aliases(["silent"]),
            arg!(--posix "Disable non-POSIX extensions."),
            arg!(-s --separate "Consider files as separate rather than as a long stream."),
            arg!(--sandbox "Operate in a sandbox by disabling e/r/w commands."),
            arg!(-u --unbuffered "Load minimal input data and flush output buffers regularly."),
            Arg::new("null-data")
                .short('z')
                .long("null-data")
                .help("Separate lines by NUL characters.")
                .action(clap::ArgAction::SetTrue),
        ])
}

// Iterate through script and file arguments specified in matches and
// return vectors of all scripts and input files in the specified order.
// If no script is specified fail with "missing script" error.
fn get_scripts_files(matches: &ArgMatches) -> UResult<(Vec<ScriptValue>, Vec<PathBuf>)> {
    let mut indexed_scripts: Vec<(usize, ScriptValue)> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();

    let script_through_options =
        // The specification of a script: through a string or a file.
        matches.contains_id("expression") || matches.contains_id("script-file");

    if script_through_options {
        // Second and third POSIX usage cases; clap script arg is actually an input file
        // sed [-En] -e script [-e script]... [-f script_file]... [file...]
        // sed [-En] [-e script]... -f script_file [-f script_file]... [file...]
        if let Some(val) = matches.get_one::<String>("script") {
            files.push(PathBuf::from(val.to_owned()));
        }
    } else {
        // First POSIX spec usage case; script is the first arg.
        // sed [-En] script [file...]
        if let Some(val) = matches.get_one::<String>("script") {
            indexed_scripts.push((0, ScriptValue::StringVal(val.to_owned())));
        } else {
            return Err(UUsageError::new(1, "missing script"));
        }
    }

    // Capture -e occurrences (STRING)
    if let Some(indices) = matches.indices_of("expression") {
        for (idx, val) in indices.zip(matches.get_many::<String>("expression").unwrap_or_default())
        {
            indexed_scripts.push((idx, ScriptValue::StringVal(val.to_owned())));
        }
    }

    // Capture -f occurrences (FILE)
    if let Some(indices) = matches.indices_of("script-file") {
        for (idx, val) in indices.zip(
            matches
                .get_many::<PathBuf>("script-file")
                .unwrap_or_default(),
        ) {
            indexed_scripts.push((idx, ScriptValue::PathVal(val.to_owned())));
        }
    }

    // Sort by index to preserve argument order.
    indexed_scripts.sort_by_key(|k| k.0);
    // Keep only the values.
    let scripts = indexed_scripts
        .into_iter()
        .map(|(_, value)| value)
        .collect();

    let rest_files: Vec<PathBuf> = matches
        .get_many::<PathBuf>("file")
        .unwrap_or_default()
        .cloned()
        .collect();
    if !rest_files.is_empty() {
        files.extend(rest_files);
    }

    // Read from stdin if no file has been specified.
    if files.is_empty() {
        files.push(PathBuf::from("-"));
    }

    Ok((scripts, files))
}

// Parse CLI flag arguments and return a ProcessingContext struct based on them
fn build_context(matches: &ArgMatches) -> ProcessingContext {
    ProcessingContext {
        all_output_files: matches.get_flag("all-output-files"),
        debug: matches.get_flag("debug"),
        regex_extended: matches.get_flag("regexp-extended"),
        follow_symlinks: matches.get_flag("follow-symlinks"),
        in_place: matches.contains_id("in-place"),
        in_place_suffix: matches
            .get_one::<String>("in-place")
            .and_then(|s| if s.is_empty() { None } else { Some(s.clone()) }),
        length: matches.get_one::<u32>("length").map_or(70, |v| *v as usize),
        quiet: matches.get_flag("quiet"),
        posix: matches.get_flag("posix"),
        separate: matches.get_flag("separate"),
        sandbox: matches.get_flag("sandbox"),
        unbuffered: matches.get_flag("unbuffered"),
        null_data: matches.get_flag("null-data"),

        // Other context
        input_name: "<stdin>".to_string(),
        line_number: 0,
        last_address: false,
        last_line: false,
        last_file: false,
        stop_processing: false,
        saved_regex: None,
        input_action: None,
        hold: StringSpace {
            content: String::new(),
            has_newline: true,
        },
        parsed_block_nesting: 0,
        label_to_command_map: HashMap::new(),
        range_commands: Vec::new(),
        substitution_made: false,
        append_elements: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*; // Allows access to private functions/items in this module

    // get_scripts_files

    // Helper function for supplying arguments
    fn get_test_matches(args: &[&str]) -> ArgMatches {
        uu_app().get_matches_from(["myapp"].iter().chain(args.iter()))
    }

    #[test]
    fn test_script_as_first_argument() {
        let matches = get_test_matches(&["1d", "file1.txt"]);
        let (scripts, files) = get_scripts_files(&matches).expect("Should succeed");

        assert_eq!(scripts, vec![ScriptValue::StringVal("1d".to_string())]);
        assert_eq!(files, vec![PathBuf::from("file1.txt")]);
    }

    #[test]
    fn test_expression_argument() {
        let matches = get_test_matches(&["-e", "s/foo/bar/", "file1.txt"]);
        let (scripts, files) = get_scripts_files(&matches).expect("Should succeed");

        assert_eq!(
            scripts,
            vec![ScriptValue::StringVal("s/foo/bar/".to_string())]
        );
        assert_eq!(files, vec![PathBuf::from("file1.txt")]);
    }

    #[test]
    fn test_script_file_argument() {
        let matches = get_test_matches(&["-f", "script.sed", "file1.txt"]);
        let (scripts, files) = get_scripts_files(&matches).expect("Should succeed");

        assert_eq!(
            scripts,
            vec![ScriptValue::PathVal(PathBuf::from("script.sed"))]
        );
        assert_eq!(files, vec![PathBuf::from("file1.txt")]);
    }

    #[test]
    fn test_multiple_files() {
        let matches = get_test_matches(&["-e", "s/foo/bar/", "file1.txt", "file2.txt"]);
        let (scripts, files) = get_scripts_files(&matches).expect("Should succeed");

        assert_eq!(
            scripts,
            vec![ScriptValue::StringVal("s/foo/bar/".to_string())]
        );
        assert_eq!(
            files,
            vec![PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]
        );
    }

    #[test]
    fn test_multiple_files_script() {
        let matches = get_test_matches(&["s/foo/bar/", "file1.txt", "file2.txt"]);
        let (scripts, files) = get_scripts_files(&matches).expect("Should succeed");

        assert_eq!(
            scripts,
            vec![ScriptValue::StringVal("s/foo/bar/".to_string())]
        );
        assert_eq!(
            files,
            vec![PathBuf::from("file1.txt"), PathBuf::from("file2.txt")]
        );
    }

    #[test]
    fn test_stdin_when_no_files() {
        let matches = get_test_matches(&["-e", "s/foo/bar/"]);
        let (scripts, files) = get_scripts_files(&matches).expect("Should succeed");

        assert_eq!(
            scripts,
            vec![ScriptValue::StringVal("s/foo/bar/".to_string())]
        );
        assert_eq!(files, vec![PathBuf::from("-")]); // Stdin should be used
    }

    #[test]
    fn test_stdin_when_no_files_script() {
        let matches = get_test_matches(&["s/foo/bar/"]);
        let (scripts, files) = get_scripts_files(&matches).expect("Should succeed");

        assert_eq!(
            scripts,
            vec![ScriptValue::StringVal("s/foo/bar/".to_string())]
        );
        assert_eq!(files, vec![PathBuf::from("-")]); // Stdin should be used
    }

    // build_context
    fn test_matches(args: &[&str]) -> ArgMatches {
        uu_app().get_matches_from(["sed"].into_iter().chain(args.iter().copied()))
    }

    #[test]
    fn test_defaults() {
        let matches = test_matches(&[]);
        let ctx = build_context(&matches);

        assert!(!ctx.all_output_files);
        assert!(!ctx.debug);
        assert!(!ctx.regex_extended);
        assert!(!ctx.follow_symlinks);
        assert!(!ctx.in_place);
        assert_eq!(ctx.in_place_suffix, None);
        assert_eq!(ctx.length, 70);
        assert!(!ctx.quiet);
        assert!(!ctx.posix);
        assert!(!ctx.separate);
        assert!(!ctx.sandbox);
        assert!(!ctx.unbuffered);
        assert!(!ctx.null_data);
    }

    #[test]
    fn test_all_flags() {
        let matches = test_matches(&[
            "--all-output-files",
            "--debug",
            "-E",
            "--follow-symlinks",
            "-i",
            "-l",
            "80",
            "-n",
            "--posix",
            "-s",
            "--sandbox",
            "-u",
            "-z",
        ]);

        let ctx = build_context(&matches);

        assert!(ctx.all_output_files);
        assert!(ctx.debug);
        assert!(ctx.regex_extended);
        assert!(ctx.follow_symlinks);
        assert!(ctx.in_place);
        assert!(ctx.in_place_suffix.is_none());
        assert_eq!(ctx.length, 80);
        assert!(ctx.quiet);
        assert!(ctx.posix);
        assert!(ctx.separate);
        assert!(ctx.sandbox);
        assert!(ctx.unbuffered);
        assert!(ctx.null_data);
    }

    #[test]
    fn test_multiple_same_arguments() {
        let matches = test_matches(&["-E", "-r"]);
        let ctx = build_context(&matches);

        assert!(ctx.regex_extended);
    }

    // Helper matching what uumain does: preprocess args, then parse with clap.
    // Needed because `test_matches` calls `uu_app().get_matches_from(...)`
    // directly, which bypasses the `-iSUFFIX` → `-i=SUFFIX` rewriting.
    fn test_matches_preprocessed(args: &[&str]) -> ArgMatches {
        use std::ffi::OsString;
        let argv: Vec<OsString> = std::iter::once(OsString::from("sed"))
            .chain(args.iter().map(OsString::from))
            .collect();
        uu_app().get_matches_from(preprocess_in_place_suffix(argv.into_iter()))
    }

    #[test]
    fn test_in_place_with_attached_short_suffix() {
        // `-iSUFFIX` (attached, no `=`) is the GNU form. The argv preprocessor
        // rewrites it to `-i=SUFFIX` so clap accepts it.
        let matches = test_matches_preprocessed(&["-i.bak"]);
        let ctx = build_context(&matches);

        assert!(ctx.in_place);
        assert_eq!(ctx.in_place_suffix, Some(".bak".to_string()));
    }

    #[test]
    fn test_in_place_with_equals_suffix() {
        let matches = test_matches_preprocessed(&["-i=.bak"]);
        let ctx = build_context(&matches);

        assert!(ctx.in_place);
        assert_eq!(ctx.in_place_suffix, Some(".bak".to_string()));

        let matches = test_matches_preprocessed(&["--in-place=.bak"]);
        let ctx = build_context(&matches);

        assert!(ctx.in_place);
        assert_eq!(ctx.in_place_suffix, Some(".bak".to_string()));
    }

    #[test]
    fn test_in_place_bare_has_no_suffix() {
        let matches = test_matches_preprocessed(&["-i"]);
        let ctx = build_context(&matches);

        assert!(ctx.in_place);
        assert_eq!(ctx.in_place_suffix, None);
    }

    #[test]
    fn test_in_place_detached_suffix_is_rejected() {
        // `-i SUFFIX` (space-separated) must not consume SUFFIX as the backup
        // extension. Here, `-i` takes no value (default_missing_value = "") and
        // `s/foo/bar/` falls through to the `[script]` positional, with
        // `file.txt` as the file. This is the primary bug being fixed: without
        // `require_equals`, clap would greedily consume `s/foo/bar/` as SUFFIX
        // and then try to compile `file.txt` as a sed script.
        let matches = test_matches_preprocessed(&["-i", "s/foo/bar/", "file.txt"]);
        let (scripts, files) = get_scripts_files(&matches).expect("should succeed");

        assert_eq!(
            scripts,
            vec![ScriptValue::StringVal("s/foo/bar/".to_string())]
        );
        assert_eq!(files, vec![PathBuf::from("file.txt")]);

        let ctx = build_context(&matches);
        assert!(ctx.in_place);
        assert_eq!(ctx.in_place_suffix, None);
    }

    #[test]
    fn test_in_place_equals_with_empty_suffix_behaves_like_bare_i() {
        // `-i=` reaches clap untouched (the preprocessor leaves `-i=*` alone
        // because `bytes[2] == b'='`). clap accepts `-i=` as `in-place` with
        // value `""`, and `build_context` folds an empty SUFFIX string back
        // to `None` — so `-i=` is equivalent to bare `-i` (in-place edit with
        // no backup). Pinned here so a future preprocessor or clap tweak
        // can't silently drift this behavior.
        let matches = test_matches_preprocessed(&["-i=", "s/a/b/", "file.txt"]);
        let ctx = build_context(&matches);

        assert!(ctx.in_place);
        assert_eq!(ctx.in_place_suffix, None);
    }

    #[test]
    fn test_in_place_attached_letter_suffix_is_not_stacked() {
        // `-iE` must be treated as `-i` with SUFFIX `E`, not as `-i -E`,
        // matching GNU sed. The preprocessor rewrites `-iE` to `-i=E`;
        // `-E` (regexp-extended) stays off.
        let matches = test_matches_preprocessed(&["-iE"]);
        let ctx = build_context(&matches);

        assert!(ctx.in_place);
        assert_eq!(ctx.in_place_suffix, Some("E".to_string()));
        assert!(!ctx.regex_extended);
    }

    #[test]
    fn test_preprocess_leaves_other_short_options_alone() {
        // The preprocessor must only touch args beginning with `-i`. Other
        // short options, including short-option stacks like `-En`, must
        // reach clap verbatim.
        use std::ffi::OsString;
        let argv: Vec<OsString> = ["sed", "-En", "s/a/b/", "file.txt"]
            .iter()
            .map(OsString::from)
            .collect();
        let out: Vec<String> = preprocess_in_place_suffix(argv.into_iter())
            .into_iter()
            .map(|s| s.into_string().unwrap())
            .collect();
        assert_eq!(out, vec!["sed", "-En", "s/a/b/", "file.txt"]);

        let matches = test_matches_preprocessed(&["-En", "s/a/b/", "file.txt"]);
        let ctx = build_context(&matches);
        assert!(ctx.regex_extended);
        assert!(ctx.quiet);
    }

    #[test]
    fn test_preprocess_rewrites_i_dot_bak_before_terminator() {
        // Positive counterpart to the terminator test: `-i.bak` appearing
        // before `--` still gets rewritten. Pins the "rewrite applies up
        // to `--`, stops after" behavior.
        use std::ffi::OsString;
        let argv: Vec<OsString> = ["sed", "-i.bak", "--", "-i.keep"]
            .iter()
            .map(OsString::from)
            .collect();
        let out: Vec<String> = preprocess_in_place_suffix(argv.into_iter())
            .into_iter()
            .map(|s| s.into_string().unwrap())
            .collect();
        assert_eq!(out, vec!["sed", "-i=.bak", "--", "-i.keep"]);
    }

    #[test]
    fn test_preprocess_does_not_touch_args_after_terminator() {
        // `-iFOO` appearing after `--` must be left alone (it's a positional
        // file, not an option). This matches standard POSIX argv handling.
        use std::ffi::OsString;
        let argv: Vec<OsString> = ["sed", "s/a/b/", "--", "-i.bak"]
            .iter()
            .map(OsString::from)
            .collect();
        let out: Vec<String> = preprocess_in_place_suffix(argv.into_iter())
            .into_iter()
            .map(|s| s.into_string().unwrap())
            .collect();
        assert_eq!(out, vec!["sed", "s/a/b/", "--", "-i.bak"]);
    }

    #[test]
    fn test_length_default_and_custom() {
        let matches_default = test_matches(&[]);
        let matches_custom = test_matches(&["-l", "120"]);

        let ctx_default = build_context(&matches_default);
        let ctx_custom = build_context(&matches_custom);

        assert_eq!(ctx_default.length, 70);
        assert_eq!(ctx_custom.length, 120);
    }
}
