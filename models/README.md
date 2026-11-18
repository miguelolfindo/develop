# Models

## Face detector

`face_detection_yunet_2023mar.onnx` finds faces: for People (face recognition), for Auto and Skin
white balance, and for the skin-tone reading in the MCP server's `check_photo` (see `src/skin.rs`).

- **Source:** [opencv_zoo](https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet),
  `models/face_detection_yunet/face_detection_yunet_2023mar.onnx`, unchanged (232,589 bytes;
  SHA-256 `8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4`).
- **Model:** YuNet, by Shiqi Yu and others (*YuNet: A Tiny Millisecond-level Face Detector*,
  Machine Intelligence Research, 2023).
- **License:** MIT (Copyright (c) 2020 Shiqi Yu), which allows use in this GPL program; the full
  text is in `LICENSE-YuNet.txt` and ships with the model.

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

# Face recognition model

`face_recognition_sface_2021dec.onnx` turns each face YuNet finds into a fingerprint of 128
numbers, so faces of the same person can be grouped (see `src/faces.rs`, `src/people.rs`).

- **Source:** [opencv_zoo](https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface),
  `models/face_recognition_sface/face_recognition_sface_2021dec.onnx`, unchanged (38,696,353
  bytes; SHA-256 `0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79`).
- **Model:** SFace, by Yaoyao Zhong and others (*SFace: Sigmoid-Constrained Hypersphere Loss for
  Robust Face Recognition*, IEEE Transactions on Image Processing, 2021).
- **License:** Apache-2.0; the full text is in `LICENSE-SFace.txt` and ships with the model.
- **Not in the repository** (too large): the release build downloads it and checks the SHA-256.
  To build with it yourself, download it from the link above into this folder.

# Image understanding model (CLIP)

Keyword suggestions and searching by description use CLIP ViT-B/32 (see `src/clip.rs`): one model
turns a photo into 512 numbers, the other turns words into numbers on the same scale, and photos
and words that match end up close together.

| File | From [Xenova/clip-vit-base-patch32](https://huggingface.co/Xenova/clip-vit-base-patch32) | SHA-256 |
|---|---|---|
| `clip_vision_quantized.onnx` | `onnx/vision_model_quantized.onnx` (89,117,001 bytes) | `583fd1110a514667812fee7d684952aaf82a99b959760c8d7dca7e0ab9839299` |
| `clip_text_quantized.onnx` | `onnx/text_model_quantized.onnx` (64,504,507 bytes) | `73baab855d406190da9faa498cfedf65f15cf309f4cc7385b7b032e6d08e5c3a` |
| `clip_tokenizer.json` | `tokenizer.json` | `f7f3b7af117d467b58374797691a6438d3e6b9e9cef800dfd5dced7f697a90cd` |
| `clip_config.json` | `config.json` | `493ef57ff783e42d1530c91b53469b7fdf8db8a9c1408e86998fcb7899a4f495` |
| `clip_preprocessor_config.json` | `preprocessor_config.json` | `6f638fb9401a6d6296feff533ee7efe657b787c49f954f82f5906b36ef2a1b1f` |

- **Model:** CLIP, by OpenAI (*Learning Transferable Visual Models From Natural Language
  Supervision*, Radford and others, 2021), converted to ONNX and quantized by Xenova.
- **License:** MIT; the full text is in `LICENSE-CLIP.txt` and ships with the model.
- **The two `.onnx` files are not in the repository** (too large): the release build downloads
  them and checks the SHA-256. To build with them yourself, download them from the links above
  into this folder with the names in the first column. The three small `.json` files are in the
  repository.
