# AI noise reduction models

Develop's AI noise reduction uses the RawNIND models, as packaged by darktable:

| File | Used for |
|---|---|
| `rawnind_bayer.onnx` | Bayer raw files (Sony, Nikon, Canon, Panasonic, most others) |
| `rawnind_linear.onnx` | Fujifilm X-Trans raws and linear DNGs (phones), after demosaicing |

- **Source:** `rawdenoise-nind.dtmodel` from the
  [darktable-ai release 5.6.0](https://github.com/darktable-org/darktable-ai/releases/tag/release-5.6.0)
  (SHA-256 of the archive: `d71b5f1e727c85a359e6f74dca9e2016c9d8fc3e2f7ac3e9b347d80ceca969af`).
  The two files here are `model_bayer.onnx` and `model_linear.onnx` from that archive,
  unchanged, renamed.
- **Model:** RawNIND (UtNet2), from *Learning Joint Denoising, Demosaicing, and Compression
  from the Raw Natural Image Noise Dataset*, Brummer & De Vleeschouwer, 2025
  ([arXiv:2501.08924](https://arxiv.org/abs/2501.08924);
  [training code](https://github.com/trougnouf/rawnind_jddc)).
- **License:** GNU GPL v3.0, as stated on darktable-ai's model card. The training data is
  listed there as CC BY 4.0 / CC0. The full GPL text is in [LICENSE](../LICENSE).
