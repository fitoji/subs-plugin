# ADR-0002: Streaming architecture — `clip_timestamps` vs. re-transcription

**Fecha:** 2026-07-06

**Contexto:**
La primera implementación de streaming re-transcribía el buffer completo en cada llamada, luego intentaba deduplicar palabras ya enviadas usando `word_timestamps`. Esto tenía un bug crítico: los timestamps de Whisper son posiciones de inferencia, no timestamps de reloj. El mismo audio transcrito dentro de un buffer más grande produce timestamps desplazados por hasta 2.84 segundos, haciendo imposible el dedup por timestamp.

**Decisión:**
Usar `clip_timestamps=[_COMMITTED_END_S, buffer_duration]` para decodificar cada región de audio exactamente UNA VEZ. El encoder de Whisper sigue viendo el buffer completo (contexto acústico), pero el decoder solo procesa la región no transcrita.

**Consecuencias:**
- Positivas: cada palabra aparece una sola vez en el transcript, sin duplicados ni gaps. Los timestamps son estables porque cada región se decodifica una vez.
- Positivas: menor cómputo — se pasa menos audio por el decoder (la parte más pesada de la inferencia).
- Negativas: `clip_timestamps` no acepta `None` como fin — requiere un segundo valor numérico > duración del buffer.
- Negativas: si Whisper no produce palabras en una región (silencio con ruido de fondo), el modelo igual procesa el mel spectrogram completo.

**Alternativas consideradas:**
1. Re-transcribir + dedup por timestamp (implementado originalmente) — roto por timestamps inconsistentes.
2. Sliding window con solapamiento — más complejo, requiere fuzzy text dedup en el overlap.
3. Transcribir solo el delta + overlap — pierde contexto acústico del encoder.
4. Prefix conditioning con `condition_on_previous_text=True` — causa loops de repetición en streaming.

**Trade-off latencia vs. fidelidad:**
- Chunks de 5 s (vs 2 s original): +3 s de latencia, ~10% mejora en WER según la literatura de Whisper-Streaming.
- Aceptado porque el overlay de subtítulos está diseñado para visualización no en vivo (el usuario mira contenido, no espera una transcripción en tiempo real milimétrica).

**Estado:** Aceptado.

**Referencias:**
- `docs/whisper-fidelity-research.md` — análisis completo
- `docs/plan-whisper-fidelity-v0.3.md` — plan de implementación
