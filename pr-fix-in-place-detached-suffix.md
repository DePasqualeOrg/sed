## Summary

Change `-i` from BSD-style (required detached SUFFIX) to GNU-style (optional attached SUFFIX). This fixes the issue reports at [#165](https://github.com/uutils/sed/issues/165) and [#229](https://github.com/uutils/sed/issues/229), at the cost of breaking BSD-compatible `-i '' -e SCRIPT` / `-i .bak -e SCRIPT` invocations.

## Tradeoff

The two dialects are syntactically incompatible for `sed -i X Y Z`:

- **BSD** (`/usr/bin/sed` on macOS/FreeBSD): `X` is SUFFIX, `Y` is the script, `Z` is the input file. `-i` *requires* a SUFFIX (use `''` for no backup).
- **GNU** (`sed` on Linux): `-i` has no SUFFIX, `X` is the script, `Y` and `Z` are input files. SUFFIX, if supplied, must be attached (`-i.bak`, `--in-place=.bak`).

The pre-fix parser used `num_args(0..=1)` with no attachment constraint, which happens to match BSD semantics. The README lists `-i` under "Supported BSD and GNU extensions" (line 102), so this was presumably intentional — but it makes the GNU form (`sed -i SCRIPT FILE`) fail with cryptic parser errors, as reported in #165 and #229.

This PR picks GNU over BSD. The motivation: GNU syntax is dominant in tutorials, Stack Overflow, scripting documentation, and LLM training data. Users and AI coding assistants produce `sed -i 's/foo/bar/' file` habitually; they rarely produce `sed -i '' -e 's/foo/bar/' file`. Aligning with the common idiom reduces user-visible friction at the cost of breaking an uncommon dialect form.

If preserving BSD compat is a requirement, a follow-up could add a mode flag (e.g., env var or `--posix`) that switches the preprocessor off.

## Problem

Running a standard in-place edit such as

```
sed -i 's/foo/bar/' file.txt
```

fails with errors like `error: extra characters at the end of the d command` or `error: unterminated substitute replacement`.

The cause is in the CLI definition (`src/sed/mod.rs`):

```rust
Arg::new("in-place")
    .short('i')
    .long("in-place")
    .num_args(0..=1)
    .default_missing_value(""),
```

With `num_args(0..=1)` and no attachment constraint, clap treats `-i SUFFIX` (space-separated) as valid and greedily consumes the next positional as the backup SUFFIX. So `sed -i 's/foo/bar/' file.txt` becomes:

- `-i` = SUFFIX `'s/foo/bar/'`
- `[script]` positional = `file.txt`

The script compiler then tries to parse `file.txt` as a sed program, which fails depending on what letters the filename happens to start with.

Neither GNU sed nor BSD sed accepts the detached form for `-i`. GNU's manpage specifies `-i[SUFFIX]` / `--in-place[=SUFFIX]`, meaning SUFFIX must be attached to `-i` (no space) or provided via `--in-place=SUFFIX`.

## Fix

Two changes in `src/sed/mod.rs`:

1. Add `.require_equals(true)` to the `in-place` Arg so clap no longer grabs the next positional as SUFFIX. With this flag, clap accepts `-i`, `-i=SUFFIX`, and `--in-place=SUFFIX`, and treats `-i SUFFIX` as `-i` followed by the positional `SUFFIX`.
2. Add a small argv preprocessor in `uumain` that rewrites `-iSUFFIX` (short form, no `=`) into `-i=SUFFIX` before clap sees the args. This preserves GNU's attached short form, which users expect. The preprocessor leaves `-i`, `-i=...`, `--in-place`, `--in-place=...`, and anything after a `--` terminator unchanged. Non-UTF-8 args pass through untouched; `build_context` already rejects them downstream via `get_one::<String>`, so there's nothing to gain by rewriting them at this layer.

Consistent with GNU sed, `-i` does not participate in short-option stacking: `-iE` is treated as `-i` with SUFFIX `E`, not as `-i -E`. Users wanting both `-i` and `-E` must spell them as separate tokens. Other short-option stacks (e.g. `-En`, `-nE`) are unaffected because the preprocessor only touches args beginning with `-i`.

The only input whose parse changes is the detached form:

| Input | Before | After |
|---|---|---|
| `-i` | in-place, no SUFFIX | unchanged |
| `-i.bak` | in-place, `.bak` SUFFIX | unchanged |
| `-i=.bak` | in-place, `.bak` SUFFIX | unchanged |
| `--in-place` | in-place, no SUFFIX | unchanged |
| `--in-place=.bak` | in-place, `.bak` SUFFIX | unchanged |
| `-i 's/a/b/' file` | **SUFFIX=`s/a/b/`, script=`file`** | in-place no SUFFIX, script=`s/a/b/`, file=`file` |

## Tests

Replaced the single `test_in_place_with_suffix` (which encoded the buggy detached form) with a set of focused tests:

- `test_in_place_with_attached_short_suffix` — `-i.bak` → `.bak` SUFFIX (via preprocessor).
- `test_in_place_with_equals_suffix` — `-i=.bak` and `--in-place=.bak` → `.bak` SUFFIX.
- `test_in_place_bare_has_no_suffix` — `-i` alone → in-place, no SUFFIX.
- `test_in_place_detached_suffix_is_rejected` — primary regression test: `-i s/foo/bar/ file.txt` resolves to script `s/foo/bar/` + file `file.txt`.
- `test_preprocess_leaves_other_short_options_alone` — `-En` passes through unchanged and yields `regex_extended=true, quiet=true`.
- `test_preprocess_rewrites_i_dot_bak_before_terminator` — `-i.bak` before `--` is rewritten; `-i.keep` after `--` is not.
- `test_preprocess_does_not_touch_args_after_terminator` — args after `--` are preserved verbatim.

Added end-to-end `in_place_edit_detached_form_treats_positional_as_script` in `tests/by-util/test_sed.rs` that runs the real binary against the original bug's argv: `sed -i s/world/universe/ file.txt` now writes the expected output instead of failing.

Updated three existing integration tests (`in_place_edit_backup`, `in_place_edit_follow_symlink_with_backup`, `in_place_edit_symlink_replaced_with_backup`) that were passing `["-i", ".bak", ...]` — the detached form that the fix intentionally rejects. They now use the GNU-standard `-i.bak` attached form.

All 284 lib tests and 172 integration tests pass (`cargo test`).
