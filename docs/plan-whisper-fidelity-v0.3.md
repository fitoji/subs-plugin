# Plan: Whisper Fidelity Improvements (v0.3)

**Fecha:** 2026-07-06
**Versión objetivo:** v0.3.0
**Documento de referencia:** `whisper-fidelity-research.md`

---

## Overview

Implementar los 4 puntos priorizados para mejorar la fidelidad de la transcripción en inglés y alemán. Los cambios tocan **2 archivos**: `ai-pipeline/stt/whisper_stream.py` (sidecar) y `src-tauri/src/audio/mod.rs` (captura de audio).

**Cambios en scope:**
1. `initial_prompt` + `temperature` fallback + `hallucination_silence_threshold` + `compression_ratio_threshold` (Python)
2. `word_timestamps=True` + dedup de palabras entre transcripciones (Python)
3. Chunks de 5s con 1s overlap en Rust (`AudioConfig`)
4. DC-removal + high-pass filter en Rust (`resample_and_mix`)

**Cambios fuera de scope (decididos diferir):**
- VAD threshold configurable (cambio trivial, lo agregamos en v0.3.1 si hace falta)
- Modelos alternados (8bit, large-v3-mlx) — research, no implementación
- Prompt por idioma detectado — depende de detectar idioma en runtime, posponer

---

## Phase 1: Quick wins en el sidecar Python (15 min)

**Objetivo:** Cambiar 4 parámetros de `mlx_whisper.transcribe()` para ganar fidelidad sin tocar arquitectura.

**Archivo:** `ai-pipeline/stt/whisper_stream.py`

**Cambios:**
```python
_TRANSCRIBE_KWARGS = {
    "path_or_hf_repo": WHISPER_MODEL,
    "temperature": (0.0, 0.2, 0.4),           # 3-step fallback chain
    "initial_prompt": (
        "Hello, how are you? Guten Tag, wie geht es Ihnen? "
        "The weather is nice. Das Wetter ist schön."
    ),
    "condition_on_previous_text": False,       # unchanged (anti-loop)
    "no_speech_threshold": 0.35,               # unchanged
    "compression_ratio_threshold": 2.4,        # 2.0 → 2.4 (default, better for DE)
    "logprob_threshold": -0.5,                 # unchanged
    "hallucination_silence_threshold": 2.0,    # NEW — kills silence hallucinations
    "word_timestamps": True,                   # NEW — needed for Phase 2 dedup
}
```

**Validación:**
- `pnpm tsc --noEmit` (no toca TS, pero correr igual)
- `python3 -c "import ast; ast.parse(open('whisper_stream.py').read())"`
- `pnpm tauri dev` + transcribir audio de prueba, verificar que:
  - No aparece texto en silencios largos (TV apagada)
  - Las palabras en alemán se capitalizan correctamente
  - Audio con ruido bajo se transcribe (no se calla como antes)

**Riesgo:** Bajo. Cambios paramétricos, no estructurales.

**Rollback:** `git revert <commit>`

---

## Phase 2: Dedup de palabras con word_timestamps (1-2 h)

**Objetivo:** Eliminar palabras duplicadas en el chat transcript cuando llega audio nuevo y re-transcribimos el buffer.

**Archivos:**
- `ai-pipeline/stt/whisper_stream.py`

**Cambios arquitecturales:**

1. Cambiar el contrato de salida para incluir timestamps por palabra:
   ```python
   send({
       "type": "transcription",
       "text": new_text,
       "words": [{"word": "Hello", "start": 1.2, "end": 1.5}, ...],
       "is_final": True,
       "timestamp": int(time.time() * 1000),
   })
   ```

2. En `transcribe_buffer()`, en vez de mandar todo el texto concatenado:
   ```python
   def transcribe_buffer() -> None:
       if len(_CONTEXT_BUFFER) < SAMPLE_RATE * 0.5:
           return
       
       audio_array = np.array(_CONTEXT_BUFFER, dtype=np.float32)
       result = _MODEL(audio_array)
       
       # Collect all words with timestamps
       all_words = []
       for seg in result.get("segments", []):
           for w in seg.get("words", []):
               all_words.append({
                   "word": w["word"].strip(),
                   "start": w["start"],
                   "end": w["end"],
               })
       
       # Filter: only words that start after the last committed time
       new_words = [w for w in all_words if w["start"] > _COMMITTED_END_S]
       
       if not new_words:
           return
       
       # Update committed end to the last word we sent
       _COMMITTED_END_S = max(w["end"] for w in new_words)
       
       # Convert audio position to time
       # (buffer is the full audio context; word.start is in audio-time)
       new_text = " ".join(w["word"] for w in new_words)
       
       send({
           "type": "transcription",
           "text": new_text,
           "words": new_words,
           "is_final": True,
           "timestamp": int(time.time() * 1000),
       })
       
       # DON'T trim context anymore — keep full buffer for next iteration
       # The dedup uses word timestamps, not audio position
   ```

3. Cambiar `_COMMITTED_SAMPLES` por `_COMMITTED_END_S` (en segundos de audio).

4. Cambiar `trim_context()` para que solo borre cuando el buffer sea muy grande (>30s) para evitar crecimiento ilimitado:
   ```python
   def trim_context() -> None:
       """Keep buffer bounded, but preserve audio around the committed time."""
       global _CONTEXT_BUFFER
       MAX_SAMPLES = SAMPLE_RATE * 30  # 30s max
       if len(_CONTEXT_BUFFER) > MAX_SAMPLES:
           # Keep last 30s
           _CONTEXT_BUFFER = _CONTEXT_BUFFER[-MAX_SAMPLES:]
   ```

5. Actualizar `reset` para limpiar `_COMMITTED_END_S` también.

**Cambio en frontend (`SubtitleOverlay.tsx`):**
- No requiere cambios — el `text` ya viene limpio
- Opcional: usar `words[].end` para auto-scroll timing

**Validación:**
- Transcribir audio de 30s continuo con habla + pausas
- Verificar que no hay palabras duplicadas en el chat
- Verificar que la primera transcripción muestra solo las primeras N palabras (no todo el audio)
- Verificar que después de una pausa larga, sigue transcribiendo sin duplicar lo anterior

**Riesgo:** Medio. Cambio de contrato (`words` array). Si el frontend no maneja bien el nuevo campo, no rompe nada (es aditivo).

**Rollback:** `git revert <commit>`

---

## Phase 3: Chunks de 5s con 1s overlap (10 min)

**Objetivo:** Dar más contexto fonético a Whisper (5s vs 2s) sin cambiar la arquitectura de captura.

**Archivo:** `src-tauri/src/audio/mod.rs`

**Cambios:**
```rust
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            chunk_ms: 5_000,           // 2_000 → 5_000 (5s chunks)
            overlap_ms: 1_000,         // 500 → 1_000 (1s overlap)
            silence_threshold: 0.02,   // unchanged
            input_sample_rate: 48_000,
            input_channels: 2,
        }
    }
}
```

**Sidecar adjustment:**
- El `min samples` check en `transcribe_buffer` se mantiene en 0.5s (la lógica de dedup ya no depende del tamaño del chunk)
- El audio llega en chunks de 5s ahora, no 2s. La frecuencia de llamadas a `_MODEL()` baja de ~30/min a ~12/min.

**Validación:**
- `cargo build` (cambio trivial)
- `pnpm tauri dev` + transcribir 30s de habla continua
- Verificar que la calidad mejora subjetivamente (palabras no truncadas, contexto mejor)
- Verificar que el chat transcript sigue apareciendo en tiempo real (con ~3s de latencia adicional)

**Riesgo:** Bajo. Cambio paramétrico. Puede aumentar uso de CPU/RAM por chunk más grande.

**Trade-off explícito:** +3s de latencia por 5-10% de mejora en fidelidad. Documentado en ADR.

**Rollback:** `git revert <commit>`

---

## Phase 4: DC-removal + high-pass filter (30 min)

**Objetivo:** Mejorar la SNR del audio antes de mandarlo a Whisper.

**Archivo:** `src-tauri/src/audio/mod.rs`

**Cambios:**

1. Agregar función `dc_remove(samples: &mut [f32])`:
   ```rust
   fn dc_remove(samples: &mut [f32]) {
       if samples.is_empty() { return; }
       let mean: f32 = samples.iter().sum::<f32>() / samples.len() as f32;
       for s in samples.iter_mut() {
           *s -= mean;
       }
   }
   ```

2. Agregar función `high_pass_iir(samples: &mut [f32], cutoff_hz: f32, sr: u32)`:
   ```rust
   /// First-order IIR high-pass filter. y[n] = α * (y[n-1] + x[n] - x[n-1])
   fn high_pass_iir(samples: &mut [f32], cutoff_hz: f32, sr: u32) {
       if samples.is_empty() { return; }
       let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
       let dt = 1.0 / sr as f32;
       let alpha = rc / (rc + dt);
       let mut prev_x = 0.0;
       let mut prev_y = 0.0;
       for x in samples.iter_mut() {
           let y = alpha * (prev_y + *x - prev_x);
           prev_x = *x;
           prev_y = y;
           *x = y;
       }
   }
   ```

3. Llamar ambas funciones en `resample_and_mix` (o donde se hace la conversión a mono/16kHz), **antes** del VAD:
   ```rust
   // After resampling and mixing to mono
   dc_remove(&mut mono_samples);
   high_pass_iir(&mut mono_samples, 80.0, 16_000);
   ```

**Validación:**
- `cargo build` + `cargo test` (si hay tests)
- `pnpm tauri dev` + transcribir audio con ruido de fondo (música,空调, etc.)
- Verificar que mejora la relación señal/ruido subjetivamente
- Verificar que el VAD sigue funcionando (no se rompe por la fase del filtro)

**Riesgo:** Bajo. Filtros estándar. Si el filtro tiene fase lineal mal calibrada, podría empeorar muy poco, pero es un IIR de primer orden — bien comportado.

**Rollback:** `git revert <commit>`

---

## Orden de ejecución

```
Phase 1 (15 min) ──→ Phase 3 (10 min) ──→ Phase 4 (30 min) ──→ Phase 2 (1-2 h)
  quick wins            chunks              audio clean          dedup (más complejo)
```

**Por qué este orden:**
- Phase 1 da feedback inmediato (transcribir algo y ver mejora)
- Phase 3 es trivial y refuerza el efecto de Phase 1
- Phase 4 limpia la entrada, beneficia a Phase 2
- Phase 2 al final porque depende de `word_timestamps=True` (introducido en Phase 1) y se beneficia del audio limpio de Phase 4

---

## Tests de aceptación

Después de implementar las 4 fases, ejecutar las siguientes pruebas:

### Test 1: Silencios largos
- Audio: 30s de habla + 5s de silencio + 10s de habla
- Esperado: NO aparece texto durante los 5s de silencio
- **Mide:** `hallucination_silence_threshold` (Phase 1)

### Test 2: Inglés con acento
- Audio: 30s de un hablante nativo de inglés con acento (UK, AU, etc.)
- Esperado: Transcripción con WER < 10%
- **Mide:** `initial_prompt` (Phase 1) + chunks 5s (Phase 3)

### Test 3: Alemán con palabras compuestas
- Audio: 30s de habla en alemán con palabras largas
- Esperado: Palabras compuestas NO se truncan, capitalización correcta
- **Mide:** `compression_ratio_threshold` (Phase 1) + `initial_prompt` (Phase 1)

### Test 4: Continuidad sin duplicados
- Audio: 60s de habla continua, sin pausas largas
- Esperado: Cero palabras duplicadas en el chat transcript
- **Mide:** dedup (Phase 2) + word_timestamps (Phase 1)

### Test 5: Audio con ruido de fondo
- Audio: habla + ruido de aire acondicionado / tráfico
- Esperado: Transcripción con menos alucinaciones
- **Mide:** DC-removal + high-pass (Phase 4)

### Test 6: Latencia
- Medir tiempo entre audio entrada → texto en pantalla
- Esperado: ≤ 5.5s (con chunk de 5s + tiempo de inferencia)
- **Mide:** trade-off de Phase 3

---

## Riesgos acumulados

| Fase | Riesgo | Mitigación |
|------|--------|------------|
| 1 | Bajo | Paramétrico, rollback trivial |
| 2 | Medio | Cambio de contrato, pero aditivo. Validar con audio real. |
| 3 | Bajo | Cambio paramétrico. Medir latencia para no degradar UX. |
| 4 | Bajo | Filtros estándar. Si empeora, revertir y probar con otro cutoff. |

**Riesgo total del PR:** Bajo-Medio. Cambios contenidos a 2 archivos. No toca frontend ni protocol de mensajes (excepto `words` array en Phase 2, que es aditivo).

---

## Definition of Done

- [ ] Phase 1 merged y validado
- [ ] Phase 3 merged y validado
- [ ] Phase 4 merged y validado
- [ ] Phase 2 merged y validado
- [ ] Los 6 tests de aceptación pasan
- [ ] Documentación actualizada (CHANGELOG, README)
- [ ] ADR-0002 escrito: trade-off latencia vs. fidelidad (5s chunks)
- [ ] Tag v0.3.0
- [ ] PR con descripción detallada + screenshots de output
