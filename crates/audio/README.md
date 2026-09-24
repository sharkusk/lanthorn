# lanthorn-audio

Sound playback for interactive fiction: synthesized bleeps plus sampled
AIFF/Ogg audio and ProTracker MOD music, built on `rodio`.

Decoding needs no output device: `decode_aiff`, `tone`/`bleep` and (with the
`mod-music` feature) `render_mod` turn a sound into `Pcm` — interleaved 16-bit
samples — and `Pcm::to_wav` wraps it in a WAV file, all without `rodio`. Build
with `default-features = false` (dropping `playback`) and nothing links an
audio device library such as ALSA.

It is the audio layer behind [lanthorn](https://github.com/sharkusk/lanthorn),
a terminal interactive-fiction player with live automapping.
