# Remote sound

> For players, the short version is in [the guide](../guide/sound.md).

lanthorn plays audio through the `audio` crate, which uses
[rodio](https://github.com/RustAudio/rodio) to open the operating system's
default audio output device **on the machine lanthorn is running on**. That
covers both the Z-machine `@sound_effect` bleeps/samples and the Glulx Glk
sound channels (Blorb AIFF/OGG/MOD resources) — same output path either way.

If you SSH into a remote box and run lanthorn there, its sound goes out the
*remote* machine's speakers, not yours. lanthorn has no built-in
network-audio feature and isn't going to grow one — routing audio off-box is
entirely an OS/transport concern, outside lanthorn's scope. This page just
points you at the standard tools for doing that.

lanthorn's own audio controls (`enable_sound` and `volume` in
`~/.lanthorn/config.toml`) only affect what lanthorn sends to the local
device — they don't change where that device is. Everything below sits
underneath that layer.

## Option 1: PulseAudio / PipeWire network forwarding

PulseAudio (and PipeWire via its `pipewire-pulse` compatibility layer) can
redirect a client's audio to a sound server running on a different machine.
Point the *remote* machine (where lanthorn runs) at your *local* machine's
sound server by setting `PULSE_SERVER` before launching lanthorn there:

```bash
# On the remote host, after tunneling (see below):
export PULSE_SERVER=tcp:localhost:4713
lanthorn path/to/story.z5
```

You need a route from the remote box back to your local PulseAudio/PipeWire
server. An SSH reverse tunnel is the simplest way to avoid exposing the audio
port to the network:

```bash
# From your LOCAL machine, forward remote port 4713 to your local PulseAudio port:
ssh -R 4713:localhost:4713 <user>@<host>
```

That assumes your local PulseAudio/PipeWire is listening on TCP (load
`module-native-protocol-tcp`, or on PipeWire enable the equivalent
`pipewire-pulse` TCP listener) and, ideally, that you've set up
`auth-anonymous=1` or cookie-based auth scoped to `localhost` so the tunnel
is the only thing exposing it. Once `PULSE_SERVER` is set and the tunnel is
up, rodio's output on the remote box lands on your local speakers exactly as
if lanthorn were running locally.

If you'd rather forward *to* the remote machine instead of tunneling back
(e.g. no SSH reverse-tunnel access), a `-L` forward with a PulseAudio server
listening remotely works the same way in reverse — just swap which host owns
`PULSE_SERVER`.

## Option 2: Remote-audio transport over SSH

These are lower-level and more manual, but useful when PulseAudio/PipeWire
forwarding isn't available (headless remote, container, minimal image).
All of them are ordinary Unix audio tools; lanthorn doesn't know or care
which one is in use — it just writes to the default output device.

**`parec`/`pacat` piped over SSH** — capture PulseAudio's monitor of the
sink lanthorn plays to on the remote host, and pipe raw PCM over SSH to
`pacat` playing on your local machine:

```bash
# On the remote host: capture the default sink's monitor, stream it out over SSH.
parec --format=s16le --rate=44100 --channels=2 | \
  ssh <user>@<local-host> pacat --format=s16le --rate=44100 --channels=2 --playback
```

(Run this the other direction — `ssh` out from the remote box to your local
sshd — if the remote can't accept inbound connections.)

**RTP** — PulseAudio also has a native RTP sender/receiver
(`module-rtp-send` / `module-rtp-recv`) for streaming a sink over the
network; tunnel the RTP/RTCP ports over SSH the same way as the native
protocol port above if you don't want it exposed directly.

**Mumble** — a low-latency voice-chat client can double as an ad hoc audio
relay: run a Mumble client on the remote host with its input set to capture
the PulseAudio monitor of the sink lanthorn uses, and listen on a client
locally connected to the same server/channel. Overkill for most setups, but
handy if you already have a Mumble server running and want to avoid raw port
forwarding.

All of these route uncompressed or lightly-compressed PCM over the network,
so expect noticeable latency (tens to hundreds of ms) and treat the
tunnel/ports as you would any other traffic leaving the box — use SSH
tunnels or otherwise restrict access rather than binding audio services to a
public interface.

## Verify it works

Once the transport is set up, load any story with sound (or a Blorb with
`Snd ` resources) and trigger a bleep or sample — or run
`/play-sound <resource-id>` in the `app` TUI to play a specific Blorb
resource on demand. If you hear it locally, the pipe is working.
