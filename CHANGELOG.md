# Changelog

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
