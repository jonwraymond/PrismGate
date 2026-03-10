# Changelog

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
