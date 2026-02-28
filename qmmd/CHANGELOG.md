# qmmd Changelog

## [v0.1.1](https://github.com/ulissesf/qmassa/releases/tag/qmmd-v0.1.1) - 2026-02-27

### Fixes

- Fix memleak when running with -f/--use-fdinfo option. (@ulissesf)
- Use maximum frequency limit not the max runtime value. (@ulissesf)

## [v0.1.0](https://github.com/ulissesf/qmassa/releases/tag/qmmd-v0.1.0) - 2026-02-25

### Features

- Initial release leveraging available qmlib drivers (@ulissesf)
  - Exports memory, engines, freqs, power, temperature and fans stats
  - Metrics exported on a Prometheus HTTP endpoint (metric names & labels defined together wth @p-zak)
  - CLI options to control which devices, update interval, IP & Port to register HTTP endpoint
  - Sample systemd service file
