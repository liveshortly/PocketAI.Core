# Third-Party Dependencies and Vendored Code

## Vendored: StableDiffusion (from mlx-swift-examples)

- **Source Repository**: [https://github.com/ml-explore/mlx-swift-examples](https://github.com/ml-explore/mlx-swift-examples)
- **Vendored Path**: `swift/Sources/PocketImageGenVendor/` (copied from `Libraries/StableDiffusion`)
- **Commit SHA**: `378f2449c257788c5067b9f8b086731d76b39b33`
- **License**: MIT License

### Rationale
`ml-explore/mlx-swift-examples` cannot currently be consumed as an SPM package dependency due to upstream version constraint mismatches:
1. Release tag `2.29.1` pins `swift-transformers` to `1.0.0..<1.1.0` and `mlx-swift` to `0.29.1..<0.30.0` (PocketCore uses `swift-transformers 1.3.5` and `mlx-swift 0.32.3`).
2. The `main` branch pins `mlx-swift` to `0.31.4..<0.32.0` (PocketCore uses `mlx-swift 0.32.3`).

Per task specification APL-15, the StableDiffusion Swift library sources are vendored into `swift/Sources/PocketImageGenVendor/` directly from commit `378f2449c257788c5067b9f8b086731d76b39b33` under the MIT License.

### License Text
```text
MIT License

Copyright (c) 2024 ml-explore

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Bundled: Pyodide 0.29.5 (core distribution)

- **Source**: [https://github.com/pyodide/pyodide/releases/tag/0.29.5](https://github.com/pyodide/pyodide/releases/tag/0.29.5), `pyodide-core-0.29.5.tar.bz2`
  (sha256 `ab08aafc11a3c9a0fe73825eb2cec78bda9464348dd17414e7ec0132ef3451e8`, matches the digest GitHub publishes for the asset).
- **Path**: `swift/Sources/PocketKit/Resources/pyodide/`: `pyodide.js`, `pyodide.asm.js`, `pyodide.asm.wasm`, `python_stdlib.zip`, `pyodide-lock.json`, `LICENSE`, unmodified.
  Not bundled: the other core files (`.mjs`, `.d.ts`, CLI entry points) and every optional package.
- **Contents**: CPython 3.13.2 compiled to WebAssembly (emscripten 4.0.9). CPython is under the PSF licence.
- **License**: Mozilla Public License 2.0 (file `LICENSE` beside the binaries). The files are shipped as-is and not modified, so MPL's source-availability duty is met by the link above.
- **Use**: AGT-13 stage 3 / AGT-14. Runs only inside the hidden web view of `WebSandbox`, offline from the app bundle; nothing is downloaded at runtime (App Review 2.5.2).
- **Size**: about 12 MB uncompressed in the bundle.
