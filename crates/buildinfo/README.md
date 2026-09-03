# lanthorn-buildinfo

A tiny build-time helper that stamps the current git commit hash into
non-release build version strings (falling back cleanly when no git checkout
is available, as in a registry build).

Used by [lanthorn](https://github.com/sharkusk/lanthorn) so a development
build and a bug report can identify themselves precisely.
