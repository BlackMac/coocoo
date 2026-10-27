// Dictation through the browser's Web Speech API (Chromium, Edge, Safari). Firefox and WebKitGTK
// (the Linux desktop shell) lack it: `supported` is false there and callers fall back to typing.

interface RecognitionResult {
  readonly isFinal: boolean;
  readonly 0: { readonly transcript: string };
}

interface RecognitionEvent extends Event {
  readonly resultIndex: number;
  readonly results: { readonly length: number; readonly [i: number]: RecognitionResult };
}

interface Recognition extends EventTarget {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  onresult: ((e: RecognitionEvent) => void) | null;
  onerror: ((e: Event & { error: string }) => void) | null;
  onend: (() => void) | null;
  start(): void;
  stop(): void;
  abort(): void;
}

type RecognitionCtor = new () => Recognition;

const w = window as unknown as { SpeechRecognition?: RecognitionCtor; webkitSpeechRecognition?: RecognitionCtor };
const Ctor = w.SpeechRecognition ?? w.webkitSpeechRecognition;

const ERRORS: Record<string, string> = {
  'not-allowed': 'Microphone access was denied.',
  'service-not-allowed': 'Speech recognition is not allowed here.',
  'audio-capture': 'No microphone found.',
  network: 'The speech service is unreachable.',
};

export class Dictation {
  readonly supported = !!Ctor;
  listening = $state(false);
  /** Words recognized so far that may still change. */
  interim = $state('');
  error = $state<string | null>(null);
  #rec: Recognition | null = null;

  /** Starts listening; every finished phrase goes to `onFinal`. */
  start(onFinal: (text: string) => void): void {
    if (!Ctor || this.listening) return;
    const rec = new Ctor();
    rec.lang = navigator.language;
    rec.continuous = true;
    rec.interimResults = true;
    rec.onresult = (e) => {
      let interim = '';
      for (let i = e.resultIndex; i < e.results.length; i++) {
        const r = e.results[i];
        if (r.isFinal) onFinal(r[0].transcript.trim());
        else interim += r[0].transcript;
      }
      this.interim = interim.trim();
    };
    rec.onerror = (e) => {
      // `no-speech` and `aborted` are not worth a message: the user simply said nothing or stopped.
      if (ERRORS[e.error]) this.error = ERRORS[e.error];
    };
    rec.onend = () => {
      if (this.#rec !== rec) return;
      this.#rec = null;
      this.listening = false;
      this.interim = '';
    };
    this.#rec = rec;
    this.error = null;
    this.listening = true;
    rec.start();
  }

  /** Stops after the current phrase is recognized (its final result still arrives). */
  stop(): void {
    this.#rec?.stop();
  }

  /** Stops at once and drops what was not recognized yet. */
  cancel(): void {
    const rec = this.#rec;
    if (!rec) return;
    this.#rec = null;
    rec.onresult = null;
    rec.abort();
    this.listening = false;
    this.interim = '';
  }
}
