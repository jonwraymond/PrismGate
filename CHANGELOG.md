# Changelog

## [1.15.0](https://github.com/jonwraymond/PrismGate/compare/v1.14.4...v1.15.0) (2026-06-01)


### Features

* add branding assets (logo SVG, favicon, OG images) ([#116](https://github.com/jonwraymond/PrismGate/issues/116)) ([eee2523](https://github.com/jonwraymond/PrismGate/commit/eee2523df578bdcdec59e28ccc9e85053ac9349d))
* **register:** improve error messages with actionable guidance ([#120](https://github.com/jonwraymond/PrismGate/issues/120)) ([a19afea](https://github.com/jonwraymond/PrismGate/commit/a19afea4660a621445a7f8e0911f34a9a8999b99))


### Bug Fixes

* bump rmcp 1.2→1.7, use AuthRequiredError::new() to fix cargo install ([#122](https://github.com/jonwraymond/PrismGate/issues/122)) ([b640355](https://github.com/jonwraymond/PrismGate/commit/b6403553b367ebdfbfb45f5206ab4889487c8c65))
* remove deprecated bitwarden ClientProjectsExt/ClientSecretsExt imports ([#128](https://github.com/jonwraymond/PrismGate/issues/128)) ([9c8fec8](https://github.com/jonwraymond/PrismGate/commit/9c8fec8859f1d89f5f2280bb230ad6de973c4a45))

## [1.14.4](https://github.com/jonwraymond/PrismGate/compare/v1.14.3...v1.14.4) (2026-04-23)


### Bug Fixes

* reap nonresponsive idle proxy clients ([eebc280](https://github.com/jonwraymond/PrismGate/commit/eebc280d6621202f2e8afb47c677cd6dc11bf369))
* reap nonresponsive idle proxy clients ([c7a3016](https://github.com/jonwraymond/PrismGate/commit/c7a3016ba8ed63c64f921846817d6bb468340be0))
* update rustls webpki advisory dependency ([0b6b899](https://github.com/jonwraymond/PrismGate/commit/0b6b89901d7a8d0a3492eed5a29ceb629ae1e9fe))

## [1.14.3](https://github.com/jonwraymond/PrismGate/compare/v1.14.2...v1.14.3) (2026-04-15)


### Bug Fixes

* detach daemon process session when spawning ([dafff8b](https://github.com/jonwraymond/PrismGate/commit/dafff8bea027647b5284359d80e39fc67ff5d197))
* detach daemon process session when spawning ([7fbd601](https://github.com/jonwraymond/PrismGate/commit/7fbd6014fbc12a1fb9dad75cefa608a838e17535))

## [1.14.2](https://github.com/jonwraymond/PrismGate/compare/v1.14.1...v1.14.2) (2026-04-15)


### Bug Fixes

* detach daemon stderr from spawning clients ([f91b491](https://github.com/jonwraymond/PrismGate/commit/f91b49175e7cc250acd7eb74ed0093d3ee3f7ca5))
* detach daemon stderr from spawning clients ([00e8f52](https://github.com/jonwraymond/PrismGate/commit/00e8f52a9b8ab72f7ec1459be517d3af18ebe12f))

## [1.14.1](https://github.com/jonwraymond/PrismGate/compare/v1.14.0...v1.14.1) (2026-04-15)


### Bug Fixes

* rebind promoted daemon socket after legacy cleanup ([7262bf1](https://github.com/jonwraymond/PrismGate/commit/7262bf1fd144cc4c60d1309ec45df873274f469a))
* rebind promoted daemon socket after legacy cleanup ([127b99e](https://github.com/jonwraymond/PrismGate/commit/127b99e4dd63ec3b0d12cec369e43a3158092736))

## [1.14.0](https://github.com/jonwraymond/PrismGate/compare/v1.13.0...v1.14.0) (2026-04-15)


### Features

* add generational daemon hot upgrade ([887370e](https://github.com/jonwraymond/PrismGate/commit/887370ed5bb78fd49300d33ab8426a5d1b878d55))
* add generational daemon hot upgrade ([a35112b](https://github.com/jonwraymond/PrismGate/commit/a35112b82c67bd9087843ca98d530cb44b2bc909))

## [1.13.0](https://github.com/jonwraymond/PrismGate/compare/v1.12.2...v1.13.0) (2026-04-14)


### Features

* add gatemini doctor diagnostics ([79bfb76](https://github.com/jonwraymond/PrismGate/commit/79bfb764d40455a82b501fa3e18227c18506545d))


### Bug Fixes

* account for dedicated pool capacity ([e2f998e](https://github.com/jonwraymond/PrismGate/commit/e2f998e4e4c933e70328c39823d2de50a3bbaaff))
* clean up backend lifecycle state on replacement ([decbcb2](https://github.com/jonwraymond/PrismGate/commit/decbcb231fabdc9c432d3bbba90fe51e7f3e523b))
* harden backend lifecycle stability ([ce44d5e](https://github.com/jonwraymond/PrismGate/commit/ce44d5e28c92576f71d95fe29ff29e46eee975f7))
* kill cli adapter process groups on timeout ([ba67505](https://github.com/jonwraymond/PrismGate/commit/ba67505128a1d12fd7dcf298c3f554705f972ed0))
* respect backend shutdown grace periods ([b74e612](https://github.com/jonwraymond/PrismGate/commit/b74e6120804030aea9f0dd1bda3e83da8f3fc5db))
* restart full dedicated backends during health recovery ([3ec624e](https://github.com/jonwraymond/PrismGate/commit/3ec624e1c858b4476b731524e2b30a5a930f4fd4))

## [1.12.2](https://github.com/jonwraymond/PrismGate/compare/v1.12.1...v1.12.2) (2026-04-14)


### Bug Fixes

* build release containers on native arm64 runners ([8be3ad7](https://github.com/jonwraymond/PrismGate/commit/8be3ad708e96756e6c5ffe5a27ec0012219a628b))

## [1.12.1](https://github.com/jonwraymond/PrismGate/compare/v1.12.0...v1.12.1) (2026-04-14)


### Bug Fixes

* unblock release builds and registry tests ([725b2e6](https://github.com/jonwraymond/PrismGate/commit/725b2e69efa284c1da0e9607e67e9fdbfb683d69))

## [1.12.0](https://github.com/jonwraymond/PrismGate/compare/v1.11.2...v1.12.0) (2026-04-14)


### Features

* harden Gatemini startup and proxy sessions ([6f6e05a](https://github.com/jonwraymond/PrismGate/commit/6f6e05a9bb7559c230f2dc74df0149473720e89d))

## [1.11.2](https://github.com/jonwraymond/PrismGate/compare/v1.11.1...v1.11.2) (2026-03-30)


### Bug Fixes

* **proxy:** add 5-minute idle timeout to prevent zombie proxy accumulation ([#79](https://github.com/jonwraymond/PrismGate/issues/79)) ([2e570e3](https://github.com/jonwraymond/PrismGate/commit/2e570e33823d9196e8eea6f926582fccac7d6193))

## [1.11.1](https://github.com/jonwraymond/PrismGate/compare/v1.11.0...v1.11.1) (2026-03-27)


### Bug Fixes

* **supervision:** circuit breaker recovery, ghost tool cleanup, retry budget ([#78](https://github.com/jonwraymond/PrismGate/issues/78)) ([7844c9b](https://github.com/jonwraymond/PrismGate/commit/7844c9b7ffc207f5ce44d44823789e05c352c5b9))

## [1.10.0](https://github.com/jonwraymond/PrismGate/compare/v1.9.0...v1.10.0) (2026-03-24)


### Features

* add realtime admin dashboard with topology view and backend cards ([61e1fc1](https://github.com/jonwraymond/PrismGate/commit/61e1fc117f645d12564ed81ae29da8172d7b8842))
* realtime admin dashboard with topology view and backend cards ([b94b567](https://github.com/jonwraymond/PrismGate/commit/b94b567d21abd83b251f6f59a4913bb2c91df9da))

## [1.9.0](https://github.com/jonwraymond/PrismGate/compare/v1.8.1...v1.9.0) (2026-03-20)


### Features

* process supervision overhaul — graceful kill, stderr capture, memory tracking ([906d107](https://github.com/jonwraymond/PrismGate/commit/906d107e002131e9bb44f884fccabb1e9d17ad77))

## [1.8.1](https://github.com/jonwraymond/PrismGate/compare/v1.8.0...v1.8.1) (2026-03-20)


### Bug Fixes

* convert thrown errors to return values in sandbox ([41d36d4](https://github.com/jonwraymond/PrismGate/commit/41d36d442f7fa7dd0e2b4855167d1b2091b5060c))
* convert thrown errors to return values in sandbox ([0544192](https://github.com/jonwraymond/PrismGate/commit/0544192e7da5be75734b78a849a4b43ff75221d9))

## [1.8.0](https://github.com/jonwraymond/PrismGate/compare/v1.7.2...v1.8.0) (2026-03-20)


### Features

* detect orphaned proxy processes and exit automatically ([df03019](https://github.com/jonwraymond/PrismGate/commit/df030199eb89b2364524c0ed3499a3a373d90d2e))
* detect orphaned proxy processes and exit automatically ([ee9d472](https://github.com/jonwraymond/PrismGate/commit/ee9d472dae51c0ceb07f5a436298d0fa72806797))

## [1.7.2](https://github.com/jonwraymond/PrismGate/compare/v1.7.1...v1.7.2) (2026-03-18)


### Bug Fixes

* add specific error hint when LLMs call meta-tools inside sandbox ([061a195](https://github.com/jonwraymond/PrismGate/commit/061a19581182b23be9e617df034da648d8ff88cc))
* add specific error hint when LLMs try to call meta-tools inside sandbox ([723cdcf](https://github.com/jonwraymond/PrismGate/commit/723cdcfa1306ad45d1cfcc33295c8b99aba61b0b))

## [1.7.1](https://github.com/jonwraymond/PrismGate/compare/v1.7.0...v1.7.1) (2026-03-18)


### Bug Fixes

* install rustls crypto provider for reqwest 0.13 ([e4fdcf7](https://github.com/jonwraymond/PrismGate/commit/e4fdcf7bde0e9a04e9269625308ae325b144d5ad))
* install rustls crypto provider for reqwest 0.13 ([b170e61](https://github.com/jonwraymond/PrismGate/commit/b170e6174eb9275701c7576101791d7533ad2553))

## [1.7.0](https://github.com/jonwraymond/PrismGate/compare/v1.6.0...v1.7.0) (2026-03-18)


### Features

* migrate rmcp 0.15 → 1.2.0, reqwest 0.12 → 0.13 ([6c0254f](https://github.com/jonwraymond/PrismGate/commit/6c0254fb0ed1a103c630bd2815c969ccc6f711c2))
* migrate rmcp 0.15 → 1.2.0, reqwest 0.12 → 0.13 ([e816dff](https://github.com/jonwraymond/PrismGate/commit/e816dfffe5dd67d7ddcb3be1cf88fb6ce0d93dae)), closes [#58](https://github.com/jonwraymond/PrismGate/issues/58)

## [1.6.0](https://github.com/jonwraymond/PrismGate/compare/v1.5.0...v1.6.0) (2026-03-16)


### Features

* optimize call_tool_chain for 6 common LLM patterns ([8680f01](https://github.com/jonwraymond/PrismGate/commit/8680f019d361bc95cbc6a89a8b7fdb2689433d34))
* optimize call_tool_chain for common LLM usage patterns ([2f244db](https://github.com/jonwraymond/PrismGate/commit/2f244dbdab9a8063d668b7744350395092cae0a5))

## [1.5.0](https://github.com/jonwraymond/PrismGate/compare/v1.4.0...v1.5.0) (2026-03-14)


### Features

* default-on output config (auto-chunk, smart truncation) ([c36f044](https://github.com/jonwraymond/PrismGate/commit/c36f0442992ef5b8022cec3278553bfc7be2e13b))
* make context efficiency features default-on with configurable OutputConfig ([b76c169](https://github.com/jonwraymond/PrismGate/commit/b76c169f6be2a423c78f859aee84a8ef40ee8db9))

## [1.4.0](https://github.com/jonwraymond/PrismGate/compare/v1.3.2...v1.4.0) (2026-03-14)


### Features

* add context efficiency features inspired by context-mode ([b62a47a](https://github.com/jonwraymond/PrismGate/commit/b62a47aefdde6cffcb486ea569670e86383bee96))
* context efficiency features (smart truncation, three-tier search, intent filtering, stats, llms.txt) ([5cd7b17](https://github.com/jonwraymond/PrismGate/commit/5cd7b17db5c473596f44ac143c80159123de6ad9))

## [1.3.2](https://github.com/jonwraymond/PrismGate/compare/v1.3.1...v1.3.2) (2026-03-14)


### Bug Fixes

* add actionable hints for common sandbox errors ([d5721ca](https://github.com/jonwraymond/PrismGate/commit/d5721caf6e59d359d985f401102eed4942a8799a))
* add actionable hints for common sandbox errors ([57c0e31](https://github.com/jonwraymond/PrismGate/commit/57c0e312d6a59614bdb44d955136435e6fa605d6))

## [1.3.1](https://github.com/jonwraymond/PrismGate/compare/v1.3.0...v1.3.1) (2026-03-13)


### Bug Fixes

* mirror backend variables onto globalThis in sandbox ([8d9d01b](https://github.com/jonwraymond/PrismGate/commit/8d9d01ba997b5e93a408cc1639113e0f81029b9c))
* mirror backend variables onto globalThis in sandbox ([6e415a3](https://github.com/jonwraymond/PrismGate/commit/6e415a346c361ae5877aa69c0038f8280912a4e7))

## [1.3.0](https://github.com/jonwraymond/PrismGate/compare/v1.2.5...v1.3.0) (2026-03-12)


### Features

* add per-session dedicated backend instances with autoscaling pool ([bae33ac](https://github.com/jonwraymond/PrismGate/commit/bae33ac28fdd031e9de37599ad45b7278ae25919))
* per-session dedicated backend instances with autoscaling pool ([d86444f](https://github.com/jonwraymond/PrismGate/commit/d86444f9455af2218ed20c1c6d0c1597c7edc459))

## [1.2.5](https://github.com/jonwraymond/PrismGate/compare/v1.2.4...v1.2.5) (2026-03-11)


### Bug Fixes

* add .trim_start_matches("return ") after existing stripping. ([05784b6](https://github.com/jonwraymond/PrismGate/commit/05784b688f1b4dfa4c13bd5032f5df70f207ae29))
* strip bare return prefix in direct tool call parser ([#43](https://github.com/jonwraymond/PrismGate/issues/43)) ([05784b6](https://github.com/jonwraymond/PrismGate/commit/05784b688f1b4dfa4c13bd5032f5df70f207ae29))

## [1.2.4](https://github.com/jonwraymond/PrismGate/compare/v1.2.3...v1.2.4) (2026-03-11)


### Bug Fixes

* use ~/.prismgate/gatemini.yaml as default config path ([#41](https://github.com/jonwraymond/PrismGate/issues/41)) ([c051ae4](https://github.com/jonwraymond/PrismGate/commit/c051ae4deda7a4a7a3816b4e0eb031ad70c74c33))

## [1.2.3](https://github.com/jonwraymond/PrismGate/compare/v1.2.2...v1.2.3) (2026-03-11)


### Bug Fixes

* emphasize schema lookup before first tool call in instructions ([#39](https://github.com/jonwraymond/PrismGate/issues/39)) ([72838f4](https://github.com/jonwraymond/PrismGate/commit/72838f4adea3437109c00b71f99d2df557b05122))

## [1.2.2](https://github.com/jonwraymond/PrismGate/compare/v1.2.1...v1.2.2) (2026-03-10)


### Bug Fixes

* rewrite overview resource as comprehensive agent instruction guide ([#37](https://github.com/jonwraymond/PrismGate/issues/37)) ([2e72dcd](https://github.com/jonwraymond/PrismGate/commit/2e72dcd33b0423c23257f6dae24125aedd62e7e2))

## [1.2.1](https://github.com/jonwraymond/PrismGate/compare/v1.2.0...v1.2.1) (2026-03-10)


### Bug Fixes

* auto-restart daemon on make install for seamless updates ([#35](https://github.com/jonwraymond/PrismGate/issues/35)) ([3727a82](https://github.com/jonwraymond/PrismGate/commit/3727a82e52c8f58a327f83dff0a7a2459849df10))

## [1.2.0](https://github.com/jonwraymond/PrismGate/compare/v1.1.1...v1.2.0) (2026-03-10)


### Features

* add __backends map for dynamic dispatch in sandbox ([#33](https://github.com/jonwraymond/PrismGate/issues/33)) ([3486fbd](https://github.com/jonwraymond/PrismGate/commit/3486fbd3f2de555f09a6dcffe4155e051abd8135))

## [1.1.1](https://github.com/jonwraymond/PrismGate/compare/v1.1.0...v1.1.1) (2026-03-10)


### Bug Fixes

* document that sandbox backends are not on globalThis ([#31](https://github.com/jonwraymond/PrismGate/issues/31)) ([b96960d](https://github.com/jonwraymond/PrismGate/commit/b96960d75bbec7450a189f1e85bdbda43e2360a0))

## [1.1.0](https://github.com/jonwraymond/PrismGate/compare/v1.0.5...v1.1.0) (2026-03-07)


### Features

* add container builds for PRs and releases ([442dd4e](https://github.com/jonwraymond/PrismGate/commit/442dd4e6b94867f1f9a6b4010f07853195b84e54))


### Bug Fixes

* clarify call_tool_chain return semantics ([6067047](https://github.com/jonwraymond/PrismGate/commit/6067047f0305847eabe4d0d59a84ff37363c52bd))

## [1.0.5](https://github.com/jonwraymond/PrismGate/compare/v1.0.4...v1.0.5) (2026-03-06)


### Bug Fixes

* document naming conventions in server instructions and prompts ([#26](https://github.com/jonwraymond/PrismGate/issues/26)) ([e5dca3b](https://github.com/jonwraymond/PrismGate/commit/e5dca3b01e5856b6133155d76ecb852752efe2a3))

## [1.0.4](https://github.com/jonwraymond/PrismGate/compare/v1.0.3...v1.0.4) (2026-03-06)


### Bug Fixes

* ad-hoc codesign macOS release binaries in CI ([#23](https://github.com/jonwraymond/PrismGate/issues/23)) ([60f85f9](https://github.com/jonwraymond/PrismGate/commit/60f85f90de7c855cb0b14284ba6c3eeb3e23d646))

## [1.0.3](https://github.com/jonwraymond/PrismGate/compare/v1.0.2...v1.0.3) (2026-03-06)


### Bug Fixes

* ad-hoc codesign binary on macOS during make install ([#21](https://github.com/jonwraymond/PrismGate/issues/21)) ([5a91f9f](https://github.com/jonwraymond/PrismGate/commit/5a91f9f8254e6444ee97608afd0371eda5a592b4))

## [1.0.2](https://github.com/jonwraymond/PrismGate/compare/v1.0.1...v1.0.2) (2026-03-06)


### Bug Fixes

* skip bare-name aliases for cached tools until backend is healthy ([#19](https://github.com/jonwraymond/PrismGate/issues/19)) ([bc00d14](https://github.com/jonwraymond/PrismGate/commit/bc00d140fa6ec1a739b5a46cbef7e361757709a8))

## [1.0.1](https://github.com/jonwraymond/PrismGate/compare/v1.0.0...v1.0.1) (2026-02-28)


### Bug Fixes

* set deterministic cwd when spawning daemon process ([#17](https://github.com/jonwraymond/PrismGate/issues/17)) ([ca04c7d](https://github.com/jonwraymond/PrismGate/commit/ca04c7de825ad898a50698c969ba1ddf563e0696))

## [1.0.0](https://github.com/jonwraymond/PrismGate/compare/v0.4.4...v1.0.0) (2026-02-27)


### ⚠ BREAKING CHANGES

* Add cli-adapter transport type for wrapping arbitrary CLI tools as MCP tool providers. This introduces a new backend type alongside stdio and streamable-http, marking the transition to 1.0.0.

### Features

* add CLI adapter backend for wrapping CLIs as MCP tools ([#15](https://github.com/jonwraymond/PrismGate/issues/15)) ([ae9d2a4](https://github.com/jonwraymond/PrismGate/commit/ae9d2a424f10224225fa3857f719e4744a5defc6))

## [0.4.4](https://github.com/jonwraymond/PrismGate/compare/v0.4.3...v0.4.4) (2026-02-26)


### Features

* resilient proxy with session replay and graceful daemon handoff ([644e721](https://github.com/jonwraymond/PrismGate/commit/644e721310083f654b66b4b9ff0dfd8809969055))

## [0.4.3](https://github.com/jonwraymond/PrismGate/compare/v0.4.2...v0.4.3) (2026-02-17)


### Bug Fixes

* move futures/futures-util to general deps and nix to unix-only ([1d4b9a4](https://github.com/jonwraymond/PrismGate/commit/1d4b9a4d3f1a0684aba79245bd6bf1ee47ae82a5))

## [0.4.2](https://github.com/jonwraymond/PrismGate/compare/v0.4.1...v0.4.2) (2026-02-17)


### Bug Fixes

* use GitHub App token for release-please to trigger release builds ([#11](https://github.com/jonwraymond/PrismGate/issues/11)) ([f9cdd09](https://github.com/jonwraymond/PrismGate/commit/f9cdd0943578055e810b8e1a52aa513df2452515))

## [0.4.1](https://github.com/jonwraymond/PrismGate/compare/v0.4.0...v0.4.1) (2026-02-17)


### Features

* automated semantic releases with release-please ([#8](https://github.com/jonwraymond/PrismGate/issues/8)) ([11d1972](https://github.com/jonwraymond/PrismGate/commit/11d19727c4a6caa8c6d7d3132b86f1b00fdfb4e6))
