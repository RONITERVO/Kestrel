# Kestrel Voice Activity Detection (VAD) & Dictation Audit Report

**Execution Timestamp:** 2026-08-21T12:49:27.920Z  
**Scenarios Evaluated:** 10 / 10 Passed (100.0%)  
**Total Simulated Audio:** 112.3s (1866 50ms frames evaluated)  

## Scenario Audit Matrix

| # | Scenario Name | Status | Auto-Stop Time | Expected Window | Timing Delta | Speech Detected | Anomalies |
|---|---|---|---|---|---|---|---|
| 1 | Standard Dictation with 2.0s Silence Auto-Stop | **✅ PASS** | 5.95s | 5.9s – 6.2s | 100ms | Yes | None |
| 2 | Natural Mid-Sentence Thinking Pause (0.7s) | **✅ PASS** | 6.65s | 6.6s – 6.9s | 100ms | Yes | None |
| 3 | Noisy Room Ambient Noise Floor (-48 dB) | **✅ PASS** | 5.45s | 5.4s – 5.7s | 100ms | Yes | None |
| 4 | Whispered Dictation with High Sensitivity (-55 dB Threshold) | **✅ PASS** | 4.45s | 4.4s – 4.7s | 100ms | Yes | None |
| 5 | Short Breath / Impulse Noise Rejection (< 400ms) | **✅ PASS** | 8.05s | 7.9s – 8.3s | 50ms | No | None |
| 6 | Delayed Speech Start (4.0s Initial Pause) | **✅ PASS** | 8.95s | 8.9s – 9.2s | 100ms | Yes | None |
| 7 | Abandoned Dictation Initial Grace Timeout (10.0s) | **✅ PASS** | 9.95s | 9.9s – 10.3s | 150ms | No | None |
| 8 | Aggressive Fast Stop (0.5s Silence Timeout) | **✅ PASS** | 3.45s | 3.4s – 3.7s | 100ms | Yes | None |
| 9 | Relaxed Long Timeout (4.0s Silence Timeout) | **✅ PASS** | 11.95s | 11.9s – 12.3s | 150ms | Yes | None |
| 10 | Provisional Checkpoints & WebM Header Preservation | **✅ PASS** | 27.95s | 27.9s – 28.3s | 150ms | Yes | None |

## Dictation Pipeline Verification

- **Live Checkpoint Advancement:** ✅ 8/8 intervals aligned to [4,8,16,24,32,48,60,92]
- **Provisional WebM Stream Concatenation:** ✅ Multi-pass header preservation verified
- **VAD State Machine Latency:** Max jitter $\le$ 50ms across all audio scenarios
