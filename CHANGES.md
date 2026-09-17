# What's in this pass

Since the "everything it needs" ask, live capture now does real protocol
decoding, not just signal-strength stats:

- **NOAA APT (live)**: streaming FM discriminator demodulates the IQ to
  audio in real time, decimated and buffered, then re-run through the
  *existing* `AptDecoder` every ~4s of new audio so the image updates as
  the pass continues.
- **ADS-B (live)**: a simplified Mode S PPM demodulator (magnitude-based
  preamble correlation + 2-samples/bit chip comparison, same idea as
  dump1090's classic decoder) turns the IQ stream into candidate 14-byte
  frames, CRC-checks each one, and only CRC-valid frames go to the
  *existing* `AdsbDecoder`. Bad candidates are dropped silently rather than
  shown as garbage output.
- **Marine AIS / HAM / FM / ISM**: still routed to the generic
  signal-strength inspector — no protocol decoder existed for these in the
  original file-based code either, so there's nothing live-specific to add
  yet without writing new protocol decoders from scratch (AIS is GMSK at
  9600 baud, a different demod path again).

## Known simplifications (real code, not placeholders, but simplified)

- ADS-B demod assumes exactly 2 samples/bit (pinned to 2.0 Msps), not
  dump1090's 2.4 Msps oversampled/interpolated approach — fine for a first
  pass, lower sensitivity on marginal signals than a production decoder.
- Only 112-bit/14-byte long frames (DF17/18 extended squitter) are
  attempted live; 56-bit short replies aren't (the file-based decoder
  above already handles both if you feed it captured frames).
- APT decimation is a plain "keep every Nth sample," no anti-alias filter.
  Should be fine given how far below Nyquist the APT subcarrier sits, but
  hasn't been checked against a noisy real-world pass.
- `AptDecoder` is re-run over the whole accumulated buffer each refresh
  rather than decoded incrementally — simplest correct thing, costs some
  CPU on a long pass, not a correctness issue.

## Still true from before

- I have no network access in this environment, so **none of this has been
  `cargo build`'d or run against real hardware.** Crate name/version
  (`rtlsdr-rs`), its Windows libusb/vcpkg linkage, and the demod code's
  actual behavior on a real dongle all need verifying on your machine or
  via the first CI run — treat this as a solid first draft, not tested.
- `build.yml`'s MSIX step assumes `Cargo.toml`/`assets/`/`packaging/` sit at
  the repo root — adjust the `Copy-Item` paths if your layout differs.
- `AppxManifest.xml` has a placeholder `Publisher` — set it to your
  certificate's exact subject before packaging, or `makeappx`/`signtool`
  will reject the build.
