# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Public API skeleton: `SstvDecoder`, `SstvEvent`, `SstvImage`, `SstvMode`,
  `ModeSpec`, `Error`, `Result`, `Resampler`.
- Mode specifications for PD120 and PD180 (timing constants translated
  from slowrx's `modespec.c`).
- Crate-level rustdoc with a runnable `## Example` snippet.
- MIT-licensed crate with ISC notice preservation for slowrx attribution.
