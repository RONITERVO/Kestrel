import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_VAD_SETTINGS,
  loadVadSettings,
  normalizeVadSettings,
  saveVadSettings,
  VoiceActivityDetector,
  type VadSettings,
} from "./voiceActivityDetection";

describe("Voice Activity Detection (VAD)", () => {
  let mockStorage: Record<string, string> = {};

  beforeEach(() => {
    mockStorage = {};
    const storageMock = {
      getItem: (key: string) => mockStorage[key] ?? null,
      setItem: (key: string, value: string) => { mockStorage[key] = value; },
      removeItem: (key: string) => { delete mockStorage[key]; },
      clear: () => { mockStorage = {}; },
    };
    Object.defineProperty(globalThis, "localStorage", {
      value: storageMock,
      writable: true,
      configurable: true,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  describe("Settings normalization & storage", () => {
    it("returns default settings when raw input is empty", () => {
      const settings = normalizeVadSettings(null);
      expect(settings).toEqual(DEFAULT_VAD_SETTINGS);
      expect(settings.enabled).toBe(true);
      expect(settings.silenceTimeoutSec).toBe(2.0);
      expect(settings.speechThresholdDb).toBe(-42);
      expect(settings.minSpeechDurationMs).toBe(400);
      expect(settings.initialGraceTimeoutSec).toBe(15.0);
    });

    it("clamps out-of-bounds user values cleanly", () => {
      const settings = normalizeVadSettings({
        enabled: true,
        silenceTimeoutSec: 100, // max is 10.0
        speechThresholdDb: -5,  // max is -20
        minSpeechDurationMs: 5000, // max is 2000
        initialGraceTimeoutSec: 2, // min is 3.0
      });

      expect(settings.silenceTimeoutSec).toBe(10.0);
      expect(settings.speechThresholdDb).toBe(-20);
      expect(settings.minSpeechDurationMs).toBe(2000);
      expect(settings.initialGraceTimeoutSec).toBe(3.0);
    });

    it("persists and loads custom settings to localStorage", () => {
      const custom: VadSettings = {
        enabled: true,
        silenceTimeoutSec: 3.5,
        speechThresholdDb: -48,
        minSpeechDurationMs: 500,
        initialGraceTimeoutSec: 20.0,
      };

      saveVadSettings(custom);
      const loaded = loadVadSettings();
      expect(loaded).toEqual(custom);
    });
  });

  describe("VoiceActivityDetector state machine", () => {
    function detectorFixture(settings: VadSettings) {
      let db = -100;
      const amplitude = () => 10 ** (db / 20);
      const analyser = {
        fftSize: 512,
        smoothingTimeConstant: 0.2,
        disconnect: vi.fn(),
        getFloatTimeDomainData: vi.fn((values: Float32Array) => values.fill(amplitude())),
      };
      const source = { connect: vi.fn(), disconnect: vi.fn() };
      class MockAudioContext {
        state = "running";
        createAnalyser() { return analyser; }
        createMediaStreamSource() { return source; }
        close = vi.fn().mockResolvedValue(undefined);
      }
      vi.stubGlobal("AudioContext", MockAudioContext);
      const onSilenceTimeout = vi.fn();
      const detector = new VoiceActivityDetector({} as MediaStream, settings, {
        onSilenceTimeout,
      });
      return { detector, onSilenceTimeout, setDb: (value: number) => { db = value; } };
    }

    it("initializes and cleans up Web Audio nodes cleanly", () => {
      const mockDisconnect = vi.fn();
      const mockClose = vi.fn().mockResolvedValue(undefined);
      const mockGetFloatTimeDomainData = vi.fn((arr: Float32Array) => arr.fill(0));

      const mockAnalyser = {
        fftSize: 512,
        smoothingTimeConstant: 0.2,
        connect: vi.fn(),
        disconnect: mockDisconnect,
        getFloatTimeDomainData: mockGetFloatTimeDomainData,
      };

      const mockSourceNode = {
        connect: vi.fn(),
        disconnect: mockDisconnect,
      };

      class MockAudioContext {
        state = "running";
        createAnalyser() { return mockAnalyser; }
        createMediaStreamSource() { return mockSourceNode; }
        close = mockClose;
      }

      vi.stubGlobal("AudioContext", MockAudioContext);

      const fakeStream = {} as MediaStream;
      const onSilenceTimeout = vi.fn();
      const onEnergyUpdate = vi.fn();

      const detector = new VoiceActivityDetector(fakeStream, DEFAULT_VAD_SETTINGS, {
        onSilenceTimeout,
        onEnergyUpdate,
      });

      detector.destroy();
      expect(mockDisconnect).toHaveBeenCalled();
      expect(mockClose).toHaveBeenCalled();
    });

    it("fires at the configured initial grace boundary when no speech begins", () => {
      vi.useFakeTimers();
      const fixture = detectorFixture({ ...DEFAULT_VAD_SETTINGS, initialGraceTimeoutSec: 3 });
      vi.advanceTimersByTime(2_950);
      expect(fixture.onSilenceTimeout).not.toHaveBeenCalled();
      vi.advanceTimersByTime(100);
      expect(fixture.onSilenceTimeout).toHaveBeenCalledOnce();
      fixture.detector.destroy();
    });

    it("arms only after minimum speech and then honors the silence boundary", () => {
      vi.useFakeTimers();
      const fixture = detectorFixture({
        ...DEFAULT_VAD_SETTINGS,
        minSpeechDurationMs: 400,
        silenceTimeoutSec: 2,
        initialGraceTimeoutSec: 10,
      });
      fixture.setDb(-20);
      vi.advanceTimersByTime(350);
      fixture.setDb(-80);
      vi.advanceTimersByTime(2_100);
      expect(fixture.onSilenceTimeout).not.toHaveBeenCalled();

      fixture.setDb(-20);
      vi.advanceTimersByTime(450);
      fixture.setDb(-80);
      vi.advanceTimersByTime(1_950);
      expect(fixture.onSilenceTimeout).not.toHaveBeenCalled();
      vi.advanceTimersByTime(100);
      expect(fixture.onSilenceTimeout).toHaveBeenCalledOnce();
      fixture.detector.destroy();
    });
  });
});
