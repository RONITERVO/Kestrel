# Kestrel Voice Activity Detection (VAD) & Dictation Audit Report

**Execution Timestamp:** 2026-08-22T19:38:06.201Z

**Scenarios Evaluated:** 10 / 10 Passed (100.0%)

**Full Scenario Timelines:** 112.3s

**Audio Evaluated Until Auto-Stop:** 93.65s (1873 50ms frames evaluated)

## Scenario Audit Matrix

| # | Scenario Name | Status | Auto-Stop Time | Expected Window | Timing Delta | Speech Detected | Anomalies |
|---|---|---|---|---|---|---|---|
| 1 | Standard Dictation with 2.0s Silence Auto-Stop | **✅ PASS** | 6.05s | 5.9s – 6.2s | 0ms | Yes | None |
| 2 | Natural Mid-Sentence Thinking Pause (0.7s) | **✅ PASS** | 6.75s | 6.6s – 6.9s | 0ms | Yes | None |
| 3 | Noisy Room Ambient Noise Floor (-48 dB) | **✅ PASS** | 5.55s | 5.4s – 5.7s | 0ms | Yes | None |
| 4 | Whispered Dictation with High Sensitivity (-55 dB Threshold) | **✅ PASS** | 4.55s | 4.4s – 4.7s | 0ms | Yes | None |
| 5 | Short Breath / Impulse Noise Rejection (< 400ms) | **✅ PASS** | 8.10s | 7.9s – 8.3s | 0ms | No | None |
| 6 | Delayed Speech Start (4.0s Initial Pause) | **✅ PASS** | 9.05s | 8.9s – 9.2s | 0ms | Yes | None |
| 7 | Abandoned Dictation Initial Grace Timeout (10.0s) | **✅ PASS** | 10.00s | 9.9s – 10.3s | 100ms | No | None |
| 8 | Aggressive Fast Stop (0.5s Silence Timeout) | **✅ PASS** | 3.55s | 3.4s – 3.7s | 0ms | Yes | None |
| 9 | Relaxed Long Timeout (4.0s Silence Timeout) | **✅ PASS** | 12.05s | 11.9s – 12.3s | 50ms | Yes | None |
| 10 | Provisional Checkpoints & WebM Header Preservation | **✅ PASS** | 28.00s | 27.9s – 28.3s | 100ms | Yes | None |

## Dictation Pipeline Verification

- **Live Checkpoint Advancement:** ✅ 8/8 intervals aligned to [4,8,16,24,32,48,60,92]
- **Provisional WebM Stream Concatenation:** ✅ Multi-pass header preservation verified
- **VAD Auto-Stop Timing Delta:** Maximum measured midpoint delta 100ms across all audio scenarios
