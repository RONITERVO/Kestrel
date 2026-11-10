// Audio-device presentation only. Sends 20 ms mono 16 kHz frames after WebView echo cancellation.
class ConversationCapture extends AudioWorkletProcessor {
  constructor() {
    super();
    this.ratio = sampleRate / 16000;
    this.phase = 0;
    this.sum = 0;
    this.count = 0;
    this.frame = new Float32Array(320);
    this.offset = 0;
  }
  process(inputs) {
    const channel = inputs[0]?.[0];
    if (!channel) return true;
    for (const sample of channel) {
      this.sum += sample;
      this.count++;
      this.phase++;
      if (this.phase >= this.ratio) {
        this.frame[this.offset++] = this.sum / this.count;
        this.sum = 0; this.count = 0; this.phase -= this.ratio;
        if (this.offset === this.frame.length) {
          this.port.postMessage(this.frame, [this.frame.buffer]);
          this.frame = new Float32Array(320); this.offset = 0;
        }
      }
    }
    return true;
  }
}
registerProcessor("kestrel-conversation-capture", ConversationCapture);
