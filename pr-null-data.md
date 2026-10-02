# sed: separate lines with NUL under `-z`, as GNU sed does

`-z` (`--null-data`) was accepted but ignored, so input and output were still split at newlines, and `sed -z 's/\n/,/g'`, a common way to join lines, changed nothing.

With `-z`, NUL now separates lines in input and output and in `N`, `D`, `G`, `H`, `P`, `W`, `R`, `w`, `=`, `F`, `i`, `c`, `l` and the output of `e`, and a newline is an ordinary character. As in GNU sed, `a` text keeps its trailing newline. Under `-z`, GNU's `M` flag treats NUL as the line break; that is not covered here.

Related changes also apply without `-z`, and match GNU sed:

- A missing separator at the end of a file's last line is now written before the next file's output, and likewise in `w` files. Under `-z` this is the common case, since text files have no terminating NUL. `Q` does not write it.
- `N` at the end of input no longer adds a newline to a last line that lacks one.
- The text of `R` and the output of the `e` command are copied unchanged, so a line that lacks a newline is no longer followed by one.
- `r` writes the separator that the last output line lacks before the file's text, even when the file is missing or empty.
- `Q` discards text queued by `a`, `r` and `R`.

`--zero-terminated` is accepted as an alias of `--null-data`, as in GNU sed.

Fixes #530.
