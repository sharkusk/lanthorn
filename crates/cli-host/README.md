# lanthorn-cli-host

Shared plumbing for lanthorn's no-map command-line players: terminal escape
handling, the input/EOF rule, an RAII terminal restore, `--help`/`--version`,
and the save-directory and multi-disk-release naming rules shared with the
main TUI.

Used by [lanthorn](https://github.com/sharkusk/lanthorn)'s `zvm-cli`,
`gvm-cli` and `scott-cli`.
