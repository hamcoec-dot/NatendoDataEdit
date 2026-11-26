Release notes waiting for the next version (move into its file, then delete this one).

### Fixes

- **Combine by title no longer loses data.** When a file had two columns with the same title, the second overwrote the first. Both are now kept, as `Note` and `Note (2)`. A file that already has a `Source file` column keeps it when you add the file-name column, which is then numbered.
- **`natendo-cli cat --by-name` keeps every column.** It had the same loss, and also silently left out columns the first file did not have. The output now has every file's columns.
- **`natendo-cli --json` works on every command.** It was ignored by `headers`, `select`, `filter`, `sort`, `dedupe`, `stats`, `count`, `sample`, `slice`, `split`, `cat`, `clean`, `convert`, `compute`, `run` and `pivot`. Scripts that already passed it will now get JSON.
