/**
 * Core subtitle event contract.
 * This interface MUST NOT change between versions without an explicit ADR migration.
 */
export interface SubtitleEvent {
  id: number;
  text: string;
  isFinal: boolean;
  timestamp: number;
}

/**
 * Extended subtitle event with translation (v0.3+).
 * Additive — never removes fields from SubtitleEvent.
 */
export interface TranslatedSubtitleEvent extends SubtitleEvent {
  translatedText: string;
  sourceLanguage: string;
  targetLanguage: string;
}

/** Dictionary lookup result (v0.4+). */
export interface DictionaryLookupResult {
  word: string;
  definition: string;
  partOfSpeech?: string;
  examples?: string[];
}

/**
 * System events for pipeline status.
 * Emitted on a separate event channel from subtitle events.
 */
export type SystemEvent =
  | { type: 'stt_status'; status: 'listening' | 'processing' | 'error' | 'reloading'; message?: string }
  | { type: 'translator_status'; status: 'ready' | 'error'; message?: string }
  | { type: 'audio_status'; status: 'active' | 'silence' | 'error'; message?: string };
