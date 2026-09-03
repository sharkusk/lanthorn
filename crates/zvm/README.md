# lanthorn-zvm

A from-scratch, zero-dependency Z-machine virtual machine — Infocom's
interactive-fiction bytecode format, versions 3 through 8 including the
graphical Version 6 titles (*Zork Zero*, *Arthur*, *Shogun*, *Journey*).
Handles execution, standard Quetzal save/restore, and the per-machine
rendering facts (screen model, palettes, fonts) that let a front end draw a
release the way its original interpreter did.

It is the engine behind [lanthorn](https://github.com/sharkusk/lanthorn), a
terminal interactive-fiction player with live automapping, and is also usable
standalone by anything that wants to run Z-machine story files.
