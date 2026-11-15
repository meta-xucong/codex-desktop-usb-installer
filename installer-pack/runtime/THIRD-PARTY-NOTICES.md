# Third-party runtime notices

## FFmpeg

The Windows package includes the BtbN FFmpeg Builds release asset `ffmpeg-n9.0-latest-win64-lgpl-shared-9.0.zip` (SHA-256 `3f67595fb8c7be0c48d8f54b2a5a89175426a6d7b194065d0c08cca9bd95eb19`). The selected artifact is labeled an LGPL shared build. Its archive carries the build's `LICENSE.txt`; retain that notice and the license files included when extracting the archive.

- Build project: https://github.com/BtbN/FFmpeg-Builds
- FFmpeg legal information: https://ffmpeg.org/legal.html
- FFmpeg Windows build links: https://ffmpeg.org/download.html

FFmpeg is a separate project. Consult the included license and upstream legal information for redistribution obligations.

## Node.js

Node.js v22.22.0 is distributed from the official Node.js release archive. The archive includes its own license and notices.

- Release archive: https://nodejs.org/dist/v22.22.0/node-v22.22.0-win-x64.zip
- Official checksum list: https://nodejs.org/dist/v22.22.0/SHASUMS256.txt

## Chrome Headless Shell

The original Chrome for Testing win64 archive for 152.0.7977.30 is bundled for HyperFrames 0.8.134. Retain all license and notice files included in the archive when extracting or redistributing it. It is deployed privately and does not replace the system browser.

- Source: https://storage.googleapis.com/chrome-for-testing-public/152.0.7977.30/win64/chrome-headless-shell-win64.zip
- Project: https://github.com/GoogleChromeLabs/chrome-for-testing

## HyperFrames CLI and Windows dependencies

HyperFrames 0.8.134 and its locked Windows x64 npm dependencies are distributed in `hyperframes-0.8.134-win-x64-node-modules.zip`. The closure is built using the bundled package.json/package-lock.json and Node 22.22.0. Preserve the complete dependency tree, including each package's LICENSE, NOTICE and other included attribution files. No npm download is required on the installation machine.

- HyperFrames package declares Apache-2.0; see its included LICENSE.
- Exact dependency versions and source tarballs: `runtime/bundled/hyperframes-package-lock.json`.

## HyperFrames private FFmpeg encoder

HyperFrames 0.8.134's default H.264 renderer uses libx264 and x264-params. Its private runtime therefore includes the original `ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip` GPL shared build. This does not replace the general LGPL FFmpeg runtime. Retain the complete archive, LICENSE.txt and other license/notice files. Do not represent this private build as LGPL.

- Build release/source recipes: https://github.com/FFmpeg-Builds/FFmpeg-Builds
- FFmpeg source: https://github.com/FFmpeg/FFmpeg
- Build identifies its exact revision in `ffmpeg -version`; archive identity is retained in media-manifest.json.
