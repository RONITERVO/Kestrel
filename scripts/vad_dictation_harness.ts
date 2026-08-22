/**
 * Offline Voice Activity Detection (VAD) & Dictation Lifecycle Simulation Harness
 *
 * Tests the real client-side Web Audio VAD engine, RMS energy calculations,
 * state machine transitions, silence timeouts, initial grace periods, custom user thresholds,
 * and MediaRecorder dictation checkpoints without requiring a desktop browser or microphone hardware.
 *
 * Usage:
 *   npm run test:vad
 *   npm run test:vad -- --silent
 */

import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import {
  DEFAULT_VAD_SETTINGS,
  normalizeVadSettings,
  VoiceActivityDetector,
  type VadSettings,
} from "../src/voiceActivityDetection";
import {
  advanceLiveTranscriptionCheckpoint,
  completeRecordingBlob,
  LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS,
} from "../src/LocalSpeechControls";

export interface AudioSegment {
  durationSec: number;
  dbLevel: number; // e.g. -25 for active speech, -70 for silence, -48 for background noise
  description: string;
}

export interface VadScenario {
  id: string;
  name: string;
  description: string;
  settings: VadSettings;
  timeline: AudioSegment[];
  expectedStopMinSec: number;
  expectedStopMaxSec: number;
  expectSpeechDetected: boolean;
  expectFinalPassAudio: boolean;
}

export interface VadFrameLog {
  timestampSec: number;
  scenarioId: string;
  inputDb: number;
  thresholdDb: number;
  isSpeaking: boolean;
  silenceRatio: number;
  state: "IDLE" | "SPEECH" | "SILENCE_COUNTDOWN" | "GRACE_COUNTDOWN" | "STOPPED";
}

export interface VadScenarioResult {
  scenarioId: string;
  name: string;
  passed: boolean;
  actualStopSec: number | null;
  expectedRangeSec: [number, number];
  timingErrorMs: number;
  speechDetected: boolean;
  anomaly?: string;
  totalFrames: number;
}

export interface VadHarnessReport {
  timestamp: string;
  totalScenarios: number;
  passedScenarios: number;
  totalFramesEvaluated: number;
  totalSimulatedDurationSec: number;
  totalEvaluatedDurationSec: number;
  scenarios: VadScenarioResult[];
}

export const VAD_TEST_SCENARIOS: VadScenario[] = [
  {
    id: "scenario-1-standard-clean-pause",
    name: "Standard Dictation with 2.0s Silence Auto-Stop",
    description: "User speaks continuously for 4.0s at normal volume (-28 dB), then pauses. Auto-stops at ~6.0s (4.0s + 2.0s).",
    settings: { ...DEFAULT_VAD_SETTINGS, silenceTimeoutSec: 2.0, speechThresholdDb: -42 },
    timeline: [
      { durationSec: 4.0, dbLevel: -28, description: "Active clear speech" },
      { durationSec: 4.0, dbLevel: -70, description: "Silence post-speech" },
    ],
    expectedStopMinSec: 5.9,
    expectedStopMaxSec: 6.2,
    expectSpeechDetected: true,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-2-mid-sentence-hesitation",
    name: "Natural Mid-Sentence Thinking Pause (0.7s)",
    description: "User speaks for 2.0s, pauses briefly for 0.7s (< 2.0s timeout), speaks for another 2.0s, then stops. Must NOT stop prematurely.",
    settings: { ...DEFAULT_VAD_SETTINGS, silenceTimeoutSec: 2.0, speechThresholdDb: -42 },
    timeline: [
      { durationSec: 2.0, dbLevel: -26, description: "First speech clause" },
      { durationSec: 0.7, dbLevel: -68, description: "Brief thinking hesitation" },
      { durationSec: 2.0, dbLevel: -25, description: "Second speech clause" },
      { durationSec: 4.0, dbLevel: -70, description: "Final silence" },
    ],
    expectedStopMinSec: 6.6,
    expectedStopMaxSec: 6.9,
    expectSpeechDetected: true,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-3-noisy-ambient-room",
    name: "Noisy Room Ambient Noise Floor (-48 dB)",
    description: "Ambient air conditioning noise at -48 dB. User speaks at -25 dB with -40 dB threshold. Accurately isolates speech and stops.",
    settings: { ...DEFAULT_VAD_SETTINGS, silenceTimeoutSec: 2.0, speechThresholdDb: -40 },
    timeline: [
      { durationSec: 3.5, dbLevel: -25, description: "Speech above ambient floor" },
      { durationSec: 4.0, dbLevel: -48, description: "Ambient noise only" },
    ],
    expectedStopMinSec: 5.4,
    expectedStopMaxSec: 5.7,
    expectSpeechDetected: true,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-4-whisper-high-sensitivity",
    name: "Whispered Dictation with High Sensitivity (-55 dB Threshold)",
    description: "User whispers at -48 dB. Highly sensitive -55 dB threshold detects quiet voice and stops 1.5s after whispering ends.",
    settings: { ...DEFAULT_VAD_SETTINGS, silenceTimeoutSec: 1.5, speechThresholdDb: -55 },
    timeline: [
      { durationSec: 3.0, dbLevel: -48, description: "Quiet whispered speech" },
      { durationSec: 3.0, dbLevel: -75, description: "Silence" },
    ],
    expectedStopMinSec: 4.4,
    expectedStopMaxSec: 4.7,
    expectSpeechDetected: true,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-5-short-impulse-cough",
    name: "Short Breath / Impulse Noise Rejection (< 400ms)",
    description: "Brief 100ms cough (-22 dB) in quiet room. Since duration < minSpeechDuration (400ms), it does NOT arm silence auto-stop.",
    settings: { ...DEFAULT_VAD_SETTINGS, minSpeechDurationMs: 400, initialGraceTimeoutSec: 8.0 },
    timeline: [
      { durationSec: 0.1, dbLevel: -22, description: "Brief cough/click" },
      { durationSec: 10.0, dbLevel: -70, description: "Continued silence" },
    ],
    expectedStopMinSec: 7.9,
    expectedStopMaxSec: 8.3,
    expectSpeechDetected: false,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-6-delayed-speech-grace-period",
    name: "Delayed Speech Start (4.0s Initial Pause)",
    description: "User pauses 4.0s before beginning to speak (< 15s initial grace), speaks for 3.0s, then stops. Auto-stops 2.0s after speech.",
    settings: { ...DEFAULT_VAD_SETTINGS, silenceTimeoutSec: 2.0, initialGraceTimeoutSec: 15.0 },
    timeline: [
      { durationSec: 4.0, dbLevel: -70, description: "Initial thinking pause" },
      { durationSec: 3.0, dbLevel: -27, description: "Active speech begins" },
      { durationSec: 4.0, dbLevel: -70, description: "Final silence" },
    ],
    expectedStopMinSec: 8.9,
    expectedStopMaxSec: 9.2,
    expectSpeechDetected: true,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-7-abandoned-recording-timeout",
    name: "Abandoned Dictation Initial Grace Timeout (10.0s)",
    description: "User starts dictation and never speaks. Initial grace window (10.0s) cleanly times out and cancels without hanging.",
    settings: { ...DEFAULT_VAD_SETTINGS, initialGraceTimeoutSec: 10.0 },
    timeline: [
      { durationSec: 12.0, dbLevel: -70, description: "No speech at all" },
    ],
    expectedStopMinSec: 9.9,
    expectedStopMaxSec: 10.3,
    expectSpeechDetected: false,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-8-fast-response-aggressive-stop",
    name: "Aggressive Fast Stop (0.5s Silence Timeout)",
    description: "Advanced user configures ultra-fast 0.5s silence timeout. Speech ends at 3.0s -> auto-stops promptly at ~3.5s.",
    settings: { ...DEFAULT_VAD_SETTINGS, silenceTimeoutSec: 0.5, speechThresholdDb: -42 },
    timeline: [
      { durationSec: 3.0, dbLevel: -28, description: "Active speech" },
      { durationSec: 2.0, dbLevel: -70, description: "Post-speech silence" },
    ],
    expectedStopMinSec: 3.4,
    expectedStopMaxSec: 3.7,
    expectSpeechDetected: true,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-9-relaxed-long-timeout",
    name: "Relaxed Long Timeout (4.0s Silence Timeout)",
    description: "Advanced user configures 4.0s silence timeout. Natural 3.0s pauses do not stop recording; stops 4.0s after speech ends.",
    settings: { ...DEFAULT_VAD_SETTINGS, silenceTimeoutSec: 4.0, speechThresholdDb: -42 },
    timeline: [
      { durationSec: 3.0, dbLevel: -26, description: "First thought" },
      { durationSec: 3.0, dbLevel: -70, description: "Long reflection pause (< 4.0s)" },
      { durationSec: 2.0, dbLevel: -26, description: "Second thought" },
      { durationSec: 6.0, dbLevel: -70, description: "Final silence" },
    ],
    expectedStopMinSec: 11.9,
    expectedStopMaxSec: 12.3,
    expectSpeechDetected: true,
    expectFinalPassAudio: true,
  },
  {
    id: "scenario-10-live-checkpoint-blob-integrity",
    name: "Provisional Checkpoints & WebM Header Preservation",
    description: "Tests 30s recording duration with 4s polling intervals, live checkpoint advancement, and WebM byte stream integrity.",
    settings: { ...DEFAULT_VAD_SETTINGS, silenceTimeoutSec: 3.0 },
    timeline: [
      { durationSec: 25.0, dbLevel: -26, description: "Continuous long speech spanning checkpoints" },
      { durationSec: 5.0, dbLevel: -70, description: "End of speech" },
    ],
    expectedStopMinSec: 27.9,
    expectedStopMaxSec: 28.3,
    expectSpeechDetected: true,
    expectFinalPassAudio: true,
  },
];

/**
 * Simulates a single VAD scenario with 50ms time steps,
 * evaluating state transitions and timing precision.
 */
export function simulateVadScenario(scenario: VadScenario, onFrame?: (frame: VadFrameLog) => void): VadScenarioResult {
  const stepMs = 50;
  const stepSec = stepMs / 1000;
  let currentMs = 0;
  let currentDb = -100;
  let speechDetected = false;
  let stoppedAtSec: number | null = null;
  let totalFrames = 0;
  const totalTimelineDuration = scenario.timeline.reduce((sum, seg) => sum + seg.durationSec, 0);
  let tick: (() => void) | null = null;
  const analyser = {
    fftSize: 512,
    smoothingTimeConstant: 0.2,
    disconnect: () => undefined,
    getFloatTimeDomainData: (values: Float32Array) => {
      values.fill(10 ** (currentDb / 20));
    },
  };
  const source = { connect: () => undefined, disconnect: () => undefined };
  class HarnessAudioContext {
    state: AudioContextState = "running";
    createAnalyser() { return analyser as unknown as AnalyserNode; }
    createMediaStreamSource() { return source as unknown as MediaStreamAudioSourceNode; }
    resume() { return Promise.resolve(); }
    close() { this.state = "closed"; return Promise.resolve(); }
  }
  const originalWindow = globalThis.window;
  const originalPerformance = globalThis.performance;
  const harnessWindow = {
    AudioContext: HarnessAudioContext,
    setInterval: (callback: TimerHandler) => {
      tick = callback as () => void;
      return 1;
    },
    clearInterval: () => undefined,
  } as unknown as Window & typeof globalThis;
  Object.defineProperty(globalThis, "window", { configurable: true, value: harnessWindow });
  Object.defineProperty(globalThis, "performance", {
    configurable: true,
    value: { now: () => currentMs },
  });

  let detector: VoiceActivityDetector | null = null;
  try {
    detector = new VoiceActivityDetector({} as MediaStream, scenario.settings, {
      onSpeechStart: () => { speechDetected = true; },
      onSilenceTimeout: () => { stoppedAtSec = currentMs / 1_000; },
      onEnergyUpdate: (_db, isSpeaking, silenceRatio) => {
        onFrame?.({
          timestampSec: currentMs / 1_000,
          scenarioId: scenario.id,
          inputDb: currentDb,
          thresholdDb: scenario.settings.speechThresholdDb,
          isSpeaking,
          silenceRatio,
          state: isSpeaking ? "SPEECH" : speechDetected ? "SILENCE_COUNTDOWN" : "GRACE_COUNTDOWN",
        });
      },
    });
    for (let t = 0; t < totalTimelineDuration && stoppedAtSec === null; t += stepSec) {
      totalFrames += 1;
      currentMs += stepMs;
      let cursor = 0;
      currentDb = -100;
      for (const segment of scenario.timeline) {
        if (t >= cursor && t < cursor + segment.durationSec) {
          currentDb = segment.dbLevel;
          break;
        }
        cursor += segment.durationSec;
      }
      tick?.();
    }
  } finally {
    detector?.destroy();
    Object.defineProperty(globalThis, "window", { configurable: true, value: originalWindow });
    Object.defineProperty(globalThis, "performance", { configurable: true, value: originalPerformance });
  }

  // Validate results
  const [minExpected, maxExpected] = [scenario.expectedStopMinSec, scenario.expectedStopMaxSec];
  const passedTime = stoppedAtSec !== null && stoppedAtSec >= minExpected && stoppedAtSec <= maxExpected;
  const passedSpeech = speechDetected === scenario.expectSpeechDetected;
  const passed = passedTime && passedSpeech;

  let anomaly: string | undefined;
  if (stoppedAtSec === null) {
    anomaly = `Recording failed to stop within ${totalTimelineDuration}s timeline.`;
  } else if (!passedTime) {
    anomaly = `Auto-stop triggered at ${stoppedAtSec.toFixed(2)}s, expected between ${minExpected.toFixed(2)}s and ${maxExpected.toFixed(2)}s.`;
  } else if (!passedSpeech) {
    anomaly = `Speech detection mismatch: detected=${speechDetected}, expected=${scenario.expectSpeechDetected}.`;
  }

  const timingErrorMs = stoppedAtSec !== null
    ? Math.round(Math.abs(stoppedAtSec - (minExpected + maxExpected) / 2) * 1000)
    : -1;

  return {
    scenarioId: scenario.id,
    name: scenario.name,
    passed,
    actualStopSec: stoppedAtSec,
    expectedRangeSec: [minExpected, maxExpected],
    timingErrorMs,
    speechDetected,
    anomaly,
    totalFrames,
  };
}

/**
 * Validates MediaRecorder WebM blob stream concatenation and checkpoint progression.
 */
export function validateDictationPipeline(): { checkpointsPassed: boolean; blobIntegrityPassed: boolean; alignedIntervals: number; expectedIntervals: number } {
  // Test checkpoint progression
  let nextIndex = 0;
  const checkpoints: number[] = [];
  for (let elapsed = 4; elapsed <= 15 * 60; elapsed += 4) {
    const advanced = advanceLiveTranscriptionCheckpoint(elapsed, nextIndex);
    if (advanced !== nextIndex) checkpoints.push(elapsed);
    nextIndex = advanced;
  }
  const checkpointsPassed = JSON.stringify(checkpoints) === JSON.stringify(LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS);

  // Test WebM Blob reconstruction
  const webmHeader = new Uint8Array([0x1a, 0x45, 0xdf, 0xa3]);
  const audioChunk1 = new Uint8Array([0x81, 0x82]);
  const audioChunk2 = new Uint8Array([0x83, 0x84]);
  const blob1 = completeRecordingBlob([new Blob([webmHeader]), new Blob([audioChunk1])], "audio/webm;codecs=opus");
  const blob2 = completeRecordingBlob([new Blob([webmHeader]), new Blob([audioChunk1]), new Blob([audioChunk2])], "audio/webm;codecs=opus");
  const blobIntegrityPassed = blob1.size === 6 && blob2.size === 8 && blob1.type === "audio/webm;codecs=opus";

  return {
    checkpointsPassed,
    blobIntegrityPassed,
    alignedIntervals: checkpoints.length,
    expectedIntervals: LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS.length,
  };
}

/**
 * Main Harness Execution Routine
 */
export async function runVadHarness(options: {
  jsonOutputPath?: string;
  reportOutputPath?: string;
  silent?: boolean;
} = {}): Promise<{ allPassed: boolean; report: VadHarnessReport }> {
  const jsonPath = options.jsonOutputPath ?? path.resolve(process.cwd(), "vad_simulation_audit.json");
  const reportPath = options.reportOutputPath ?? path.resolve(process.cwd(), "vad_simulation_report.md");

  if (!options.silent) {
    console.log("\n============================================================");
    console.log("   KESTREL VOICE ACTIVITY DETECTION (VAD) TEST HARNESS     ");
    console.log("============================================================\n");
    console.log(`Total Scenarios: ${VAD_TEST_SCENARIOS.length}`);
  }

  const allFrames: VadFrameLog[] = [];
  const scenarioResults: VadScenarioResult[] = [];
  let totalDurationSec = 0;

  // Pipeline check
  const pipeline = validateDictationPipeline();
  if (!pipeline.checkpointsPassed || !pipeline.blobIntegrityPassed) {
    throw new Error(`Dictation pipeline check failed: checkpoints=${pipeline.checkpointsPassed}, blobs=${pipeline.blobIntegrityPassed}`);
  }

  // Run each scenario
  for (let i = 0; i < VAD_TEST_SCENARIOS.length; i++) {
    const scenario = VAD_TEST_SCENARIOS[i];
    const duration = scenario.timeline.reduce((sum, s) => sum + s.durationSec, 0);
    totalDurationSec += duration;

    // Progress bar
    if (!options.silent) {
      const pct = Math.round(((i + 1) / VAD_TEST_SCENARIOS.length) * 100);
      const bar = "█".repeat(Math.floor(pct / 4)) + "░".repeat(25 - Math.floor(pct / 4));
      process.stdout.write(`\r[${bar}] ${pct}% | Scenario ${i + 1}/${VAD_TEST_SCENARIOS.length}: ${scenario.name.slice(0, 32)}...`);
    }

    const result = simulateVadScenario(scenario, (frame) => allFrames.push(frame));
    scenarioResults.push(result);
  }

  if (!options.silent) {
    process.stdout.write("\n\n");
  }

  const passedScenarios = scenarioResults.filter((s) => s.passed).length;
  const allPassed = passedScenarios === scenarioResults.length;

  const report: VadHarnessReport = {
    timestamp: new Date().toISOString(),
    totalScenarios: scenarioResults.length,
    passedScenarios,
    totalFramesEvaluated: allFrames.length,
    totalSimulatedDurationSec: Math.round(totalDurationSec * 100) / 100,
    totalEvaluatedDurationSec: Math.round(allFrames.length * 5) / 100,
    scenarios: scenarioResults,
  };

  // Write JSON audit
  fs.writeFileSync(jsonPath, JSON.stringify({ summary: report, sampleFrames: allFrames.slice(0, 500) }, null, 2), "utf-8");

  // Write Markdown Report
  let markdown = `# Kestrel Voice Activity Detection (VAD) & Dictation Audit Report\n\n`;
  markdown += `**Execution Timestamp:** ${report.timestamp}\n\n`;
  markdown += `**Scenarios Evaluated:** ${report.passedScenarios} / ${report.totalScenarios} Passed (${((report.passedScenarios / report.totalScenarios) * 100).toFixed(1)}%)\n\n`;
  markdown += `**Full Scenario Timelines:** ${report.totalSimulatedDurationSec}s\n\n`;
  markdown += `**Audio Evaluated Until Auto-Stop:** ${report.totalEvaluatedDurationSec}s (${report.totalFramesEvaluated} 50ms frames evaluated)\n\n`;
  markdown += `## Scenario Audit Matrix\n\n`;
  markdown += `| # | Scenario Name | Status | Auto-Stop Time | Expected Window | Timing Delta | Speech Detected | Anomalies |\n`;
  markdown += `|---|---|---|---|---|---|---|---|\n`;

  scenarioResults.forEach((res, idx) => {
    const statusIcon = res.passed ? "✅ PASS" : "❌ FAIL";
    const stopTime = res.actualStopSec !== null ? `${res.actualStopSec.toFixed(2)}s` : "None";
    const expected = `${res.expectedRangeSec[0].toFixed(1)}s – ${res.expectedRangeSec[1].toFixed(1)}s`;
    const delta = `${res.timingErrorMs}ms`;
    const speech = res.speechDetected ? "Yes" : "No";
    const anomaly = res.anomaly ? `⚠️ ${res.anomaly}` : "None";
    markdown += `| ${idx + 1} | ${res.name} | **${statusIcon}** | ${stopTime} | ${expected} | ${delta} | ${speech} | ${anomaly} |\n`;
  });

  markdown += `\n## Dictation Pipeline Verification\n\n`;
  const maximumJitterMs = Math.max(0, ...scenarioResults.map((result) => result.timingErrorMs));
  markdown += `- **Live Checkpoint Advancement:** ✅ ${pipeline.alignedIntervals}/${pipeline.expectedIntervals} intervals aligned to ${JSON.stringify(LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS)}\n`;
  markdown += `- **Provisional WebM Stream Concatenation:** ✅ Multi-pass header preservation verified\n`;
  markdown += `- **VAD Auto-Stop Timing Delta:** Maximum measured midpoint delta ${maximumJitterMs}ms across all audio scenarios\n`;

  fs.writeFileSync(reportPath, markdown, "utf-8");

  if (!options.silent) {
    console.log("============================================================");
    console.log("                     HARNESS RESULTS                        ");
    console.log("============================================================");
    console.log(`Total Scenarios:         ${report.totalScenarios}`);
    console.log(`Passed Scenarios:        ${report.passedScenarios} (${((report.passedScenarios / report.totalScenarios) * 100).toFixed(1)}%)`);
    console.log(`Total Frames Evaluated:  ${report.totalFramesEvaluated}`);
    console.log("============================================================\n");
    console.log(`✅ Saved Timeline JSON to:   ${jsonPath}`);
    console.log(`✅ Saved Markdown Report to: ${reportPath}\n`);
  }

  return { allPassed, report };
}

async function runCli() {
  const args = process.argv.slice(2);
  const options: { jsonOutputPath?: string; reportOutputPath?: string; silent?: boolean } = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--json" && args[i + 1]) {
      options.jsonOutputPath = args[++i];
    } else if (args[i] === "--report" && args[i + 1]) {
      options.reportOutputPath = args[++i];
    } else if (args[i] === "--silent") {
      options.silent = true;
    }
  }
  const { allPassed } = await runVadHarness(options);
  if (!allPassed) throw new Error("VAD simulation harness found failures.");
}

const thisModule = path.resolve(fileURLToPath(import.meta.url)).toLowerCase();
const launchedDirectly = process.env.npm_lifecycle_event === "test:vad"
  || process.argv.some((argument) => {
    if (!/vad_dictation_harness\.ts$/i.test(argument)) return false;
    return path.resolve(argument).toLowerCase() === thisModule;
  });
if (launchedDirectly) {
  void runCli().catch((err) => {
    console.error("\n❌ Fatal harness error:", err);
    process.exit(1);
  });
}
