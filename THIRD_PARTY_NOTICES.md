# Third-Party Notices

This notice covers the third-party assets embedded by the OPUI export, native raster export, and reference-renderer paths. Exact files, hashes, source archives, and closure exclusions are recorded in `OPUI_RC2_ASSETS.json`.

## Iconify catalogs

`crates/op-editor-ui/assets/iconify-catalog-core.json` and `iconify-catalog-brands.json` are generated from exact Iconify JSON packages:

- Feather Icons: MIT, Copyright (c) 2013-2023 Cole Bemis
- Lucide: ISC, Copyright (c) 2026 Lucide Icons and Contributors; listed Feather-derived icons remain under MIT
- Simple Icons 16.15.0: CC0-1.0; trademark and patent rights are not granted by CC0

The applicable terms and attribution notices are in `crates/op-editor-ui/assets/ICONIFY-LICENSES.md`.

## Roboto

`crates/op-host-native/assets/Roboto-Regular.ttf` is the byte-identical Google Fonts Roboto v18 Latin subset at `KFOmCnqEu92Fr1Mu4mxP.ttf`, Version 2.137 (2017), Copyright 2011 Google Inc. It is licensed under Apache-2.0. The exact license text is adjacent at `crates/op-host-native/assets/Roboto-Regular.LICENSE-APACHE-2.0.txt`.

## Bundled renderer fonts

The following fonts embed OFL-1.1 copyright and license records in their name tables. The complete OFL-1.1 text is adjacent at `crates/op-host-desktop/assets/fonts/LICENSE-OFL-1.1.txt`.

- Cormorant Garamond, Copyright 2015 The Cormorant Project Authors
- DM Mono Medium and Regular, Copyright 2020 The DM Mono Project Authors
- DM Sans, Copyright 2014 The DM Sans Project Authors
- DM Serif Display, Copyright 2014-2017 Adobe Systems Incorporated and Copyright 2019 Google LLC
- Instrument Serif, Copyright 2022 The Instrument Serif Project Authors
- Inter, Copyright 2016 The Inter Project Authors
- JetBrains Mono, Copyright 2020 The JetBrains Mono Project Authors
- Manrope, Copyright 2019 The Manrope Project Authors
- Outfit, Copyright 2021 The Outfit Project Authors
- Space Grotesk, Copyright 2020 The Space Grotesk Project Authors

## CanvasKit

CanvasKit is outside the native RC2 exporter/reference-renderer closure but remains verified for the wider OpenPencil application. `crates/op-host-web/assets/canvaskit/{canvaskit.js,canvaskit.wasm,LICENSE}` match `canvaskit-wasm@0.40.0` byte-for-byte. The adjacent `LICENSE` is BSD-3-Clause.

## Outside the RC2 closure

The checked-in `pkg-ck`, `pkg-webgl`, and `screenshot` trees are not compiled, installed, executed, or required by the RC2 exporter or reference renderer. They remain unchanged as whole-application/editor/marketing material and are excluded from the RC2 Nix source input. This notice makes no origin or license claim for those unrelated files.
