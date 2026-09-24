# sharpless

Rounded screen corners for Wayland.

- Works on any compositor with `wlr-layer-shell` (Hyprland, Sway, river, ...)
- Anti-aliased corners
- Squircles are supported

## Running

Requires a live Wayland session.

With Nix (no clone needed):

```sh
nix run github:Aphyra0/sharpless
```

## Command-line options

```
Draw black rounded corners at the edges of the screen on Wayland

Usage: sharpless [OPTIONS]

Options:
      --radius <RADIUS>        Corner radius in logical pixels [default: 6]
      --curvature <CURVATURE>  Squircle strength: 2.0 is a circular arc, larger values flatten the edges and tighten the turn [default: 4]
  -h, --help                   Print help
  -V, --version                Print version
```

- `--radius` — how far from each screen corner the mask extends, in logical
  (not physical) pixels.
- `--curvature` — superellipse exponent of the corner arc. `2.0` is a plain
  circular corner; larger values (e.g. the default `4.0`) flatten the straight
  edges and tighten the turn, giving an iOS-style squircle.

Example: big soft corners

```sh
sharpless --radius 12 --curvature 5
```

## Developing

```sh
nix develop . 
cargo run
```

## License

MIT. See [LICENSE](LICENSE).

## Authorship disclosure

All code in this repository was written by an AI — an open-source-weights
model hosted in the European Union. No code was written by a human
author.
