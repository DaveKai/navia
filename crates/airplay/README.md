# navia-airplay

AirPlay 2 protocol code, copied from [airplay2-rs](https://github.com/lmcgartland/airplay2-rs) by Lucas McGartland
and contributors at commit `e2e63e2`. GPL-3.0-or-later. Navia's sender is in `navia-core` (`audio::airplay`).

## Kept

| Upstream crate | Module | Contents |
| --- | --- | --- |
| `airplay-core` | `base` | Devices, feature bits, codecs, stream config, errors |
| `airplay-crypto` | `crypto` | SRP, Ed25519/X25519, HKDF, ChaCha20-Poly1305, AES, TLV8 |
| `airplay-pairing` | `pairing` | HomeKit pair-setup (PIN and transient) and pair-verify |
| `airplay-rtsp` | `rtsp` | Requests, responses, the encrypted connection, SETUP plists, DAAP, SDP |
| `airplay-timing` | `timing` | The clock and the NTP timing server |
| `airplay-discovery` | `discovery` | mDNS TXT parsing. Navia browses with its own mdns-sd code |
| `airplay-audio` | `audio` | The ALAC encoder, RTP packets and the audio cipher |

## Removed

The TUI, Bluetooth input, the file decoder, the streamer and client, the equalizer, spatial audio, the resampler,
multi-room, PTP, FairPlay, legacy RSA, `EncryptedChannel` and the AAC encoder.

## Changed

- `audio::rtp::set_socket_qos` uses `socket2`, so it builds on Windows.
- `pairing::PairSetup::set_pin` takes the PIN after M1. Some receivers, such as LG TVs, only show the PIN once
  M1 arrives.
- `timing::NtpTimingServer::start_on` binds a chosen port, so one firewall rule covers it. `requests_served` says
  whether the receiver asked for timing. Dropping the server closes its socket.
- `rtsp::RtspRequest::get_parameter_text` was added, to read the receiver's volume.
- The separate crates became modules (`airplay_core::` is now `crate::base::`, and so on). Tests for removed code
  were dropped.

Fixes for all receivers are in Navia's sender instead: the sync packet's latency, not sending a volume until you
move the slider, and keeping pairings in the system keyring.
