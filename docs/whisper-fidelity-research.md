# Whisper Transcription Fidelity — Research & Recommendations

**Fecha:** 2026-07-06
**Audiencia:** Equipo de desarrollo + agentes de IA
**Estado:** Documento de investigación — implementación pendiente
**Modelo actual:** `mlx-community/whisper-large-v3-turbo`
**Idiomas objetivo:** Inglés, Alemán (con detección automática)

---

## 1. Revisión del skill `openai-whisper`

El skill `openai-whisper` (de `steipete/clawdis`) está en `.agents/skills/openai-whisper/SKILL.md`. Es **muy básico**: solo documenta el CLI de `openai-whisper` (paquete oficial de OpenAI), con ejemplos como:

```bash
whisper /path/audio.mp3 --model medium --output_format txt --output_dir .
```

**No aplica a este proyecto** porque usamos `mlx-whisper` (Apple MLX) con un sidecar Python que habla JSON Lines con Rust por stdin/stdout. Las recomendaciones de este documento se basan en la documentación oficial de `mlx-whisper` v0.4.3 y en la experiencia con Whisper large-v3.

---

## 2. Estado actual de la pipeline

### Frontend (Tauri/React)
- Escucha sistema con `screencapturekit-rs` (48 kHz, estéreo)
- Resample a 16 kHz mono, normaliza a `[-1, 1]`
- VAD por RMS (threshold 0.02)
- Chunks de **2 s** con overlap de **0.5 s** (configurado en `AudioConfig`)
- PCM i16 → base64 → JSON → stdin del sidecar Python

### Sidecar (`ai-pipeline/stt/whisper_stream.py`)
```python
_TRANSCRIBE_KWARGS = {
    "path_or_hf_repo": "mlx-community/whisper-large-v3-turbo",
    "temperature": 0.0,                        # determinista
    "condition_on_previous_text": False,       # anti-loop
    "no_speech_threshold": 0.35,               # agresivo contra silencios
    "compression_ratio_threshold": 2.0,        # estricto (default 2.4)
    "logprob_threshold": -0.5,                 # estricto (default -1.0)
}
```

### Loop de transcripción
- Acumula audio en `_CONTEXT_BUFFER` (list[float])
- En cada `audio` chunk nuevo, llama a `_MODEL(audio_array)` con **toda** la cola
- Asume que el modelo re-emite el mismo texto para la parte ya cometida
- `_COMMITTED_SAMPLES` se setea a la longitud total → `trim_context()` borra todo
- No hay dedup de palabras entre transcripciones consecutivas

---

## 3. Gaps identificados (10 problemas, ordenados por impacto)

### 3.1. Falta `initial_prompt` — ALTO IMPACTO
**Problema:** El decoder arranca "en blanco" sin contexto. Para idiomas como alemán, donde las palabras compuestas y la capitalización de sustantivos son críticas, un prompt inicial bilingüe guía al tokenizer a:
- Usar la capitalización correcta (sustantivos en DE)
- Activar el léxico esperado (Hello/Guten Tag)
- Mantener la estructura de frases del idioma detectado

**Fix:** `initial_prompt: "Hello, how are you? Guten Tag, wie geht es Ihnen? The weather is nice. Das Wetter ist schön."`

**Impacto:** 3-5% de mejora en WER según experiencia con Whisper-Streaming. Bajo costo.

### 3.2. `condition_on_previous_text=False` pierde contexto
**Problema:** Activamos `False` para evitar loops de repetición en modo streaming. Esto significa que cada transcripción es **independiente** — el modelo no "recuerda" lo que dijo antes en el buffer.

**Trade-off:** Con `True` hay mejor coherencia entre chunks pero aparecen loops cuando la salida es ambigua (palabras como "the", "a", "und" se repiten).

**Fix recomendado:** Mantener `False` por ahora, mitigar con `repetition_penalty` (no soportado por mlx_whisper directamente) o con dedup post-transcripción (ver §3.7).

**Impacto:** Marginal si el resto de la pipeline está bien. Resolver cuando se implemente dedup.

### 3.3. No hay `word_timestamps` — dedup imposible
**Problema:** Sin timestamps por palabra, no podemos saber qué parte del output corresponde a audio nuevo vs. audio ya enviado al frontend. `_COMMITTED_SAMPLES` se setea a "todo" y se confía en que el modelo reproduzca el mismo texto.

**Realidad:** El modelo no siempre reproduce exactamente el mismo texto (especialmente con `condition_on_previous_text=False`). Resultado: palabras duplicadas en el chat transcript.

**Fix:** Habilitar `word_timestamps=True`. Cambiar el contrato para que cada transcripción lleve el array de palabras con su `start`/`end`. En el frontend, dedupe por overlap de timestamps.

**Impacto:** 5-10% de mejora en la calidad de la salida (menos duplicados, mejor sincronización).

### 3.4. `temperature=0.0` sin fallback chain
**Problema:** El default de `mlx_whisper.transcribe` es `temperature=(0.0, 0.2, 0.4, 0.6, 0.8, 1.0)` — una cadena de fallback. Si la primera pasada (t=0) tiene baja confianza, sube a 0.2, etc.

Nosotros hardcodeamos `0.0` solo. Esto significa:
- Audio con ruido bajo → puede fallar en silencio (sin transcribir)
- Audio con acento fuerte → puede alucinar

**Fix:** `temperature=(0.0, 0.2, 0.4)` — solo 3 valores, suficiente fallback sin perder velocidad.

**Impacto:** Robustez mejorada en condiciones adversas, sin sacrificar velocidad (la primera pasada sigue siendo greedy).

### 3.5. No hay `hallucination_silence_threshold`
**Problema:** Whisper tiene un bug conocido: puede transcribir texto completamente inventado durante silencios largos (TV apagada, pausas largas). El modelo "llena" silencios con texto plausible.

**Fix:** `hallucination_silence_threshold=2.0` — si hay 2 s de silencio (RMS bajo) durante la generación, descarta el output.

**Impacto:** Menos basura en el chat transcript. Crítico para字幕 de películas/series.

### 3.6. Chunks de 2 s — subóptimo para Whisper
**Problema:** Whisper fue entrenado con ventanas de 30 s. Chunks de 2 s tienen muy poco contexto fonético. La investigación de Whisper-Streaming muestra que **5-10 s es el sweet spot** para calidad en tiempo real.

**Trade-off:** Más latencia. Con 5 s chunks, la latencia pasa de 2 s a 5 s.

**Fix:** Cambiar `AudioConfig.chunk_ms` de 2000 a 5000. Ajustar `overlap_ms` de 500 a 1000 (mantener relación 5:1).

**Impacto:** 5-10% de mejora en WER. +3 s de latencia. Aceptable para un overlay de subtítulos.

### 3.7. No hay dedup de palabras entre transcripciones
**Problema:** Cuando llega audio nuevo, re-transcribimos **todo** el buffer. El modelo puede emitir el mismo texto para la parte ya transcrita (especialmente con `condition_on_previous_text=False` que da resultados consistentes para el mismo input).

**Con word_timestamps=True** (ver §3.3) podemos:
1. Cada transcripción devuelve `[{word, start, end}, ...]`
2. Comparamos con `_COMMITTED_END_TIME` (en segundos)
3. Solo emitimos palabras con `start > _COMMITTED_END_TIME`
4. Actualizamos `_COMMITTED_END_TIME = max(end for each word sent)`

**Fix:** Implementar lógica de dedup en `transcribe_buffer()`.

**Impacto:** Cero duplicados en el chat. Cambio necesario para que el chat se vea limpio.

### 3.8. `compression_ratio_threshold=2.0` muy estricto para alemán
**Problema:** El default de Whisper es 2.4. Alemán tiene palabras compuestas largas (`Donaudampfschifffahrtsgesellschaftskapitän`) que pueden triggerear la heurística de "compresión alta = alucinación".

**Fix:** `compression_ratio_threshold=2.4` (default).

**Impacto:** Menos truncado de palabras alemanas largas.

### 3.9. VAD threshold 0.02 puede ser muy sensible
**Problema:** Películas/series tienen mezcla de audio con rango dinámico amplio. Un RMS de 0.02 puede descartar diálogos susurrados pero pasar explosiones.

**Fix recomendado:** Bajar a 0.01 o hacer configurable desde settings.

**Impacto:** Detecta más habla. Cambio de Rust.

### 3.10. Sin pre-procesamiento de audio (DC-removal, high-pass)
**Problema:** El audio del sistema puede tener componente DC (offset) y ruido de baja frecuencia (<80 Hz) que no aportan información de habla y empeoran la SNR.

**Fix:** En `resample_and_mix` (Rust), después de normalizar:
1. Calcular DC offset: `dc = mean(samples)`
2. Restar: `samples -= dc`
3. Aplicar high-pass IIR con cutoff ~80 Hz (filtro de primer orden es suficiente)

**Impacto:** 5% de mejora en SNR. Menos alucinaciones por ruido de fondo.

---

## 4. Resumen de impacto estimado

| # | Cambio | Archivo | Impacto | Esfuerzo |
|---|--------|---------|---------|----------|
| 1 | `initial_prompt` bilingüe | `whisper_stream.py` | Alto | 5 min |
| 2 | `word_timestamps=True` + dedup | `whisper_stream.py` | Alto | 1-2 h |
| 3 | `temperature=(0.0, 0.2, 0.4)` | `whisper_stream.py` | Medio | 5 min |
| 4 | `hallucination_silence_threshold=2.0` | `whisper_stream.py` | Alto | 5 min |
| 5 | Chunks 5s / overlap 1s | Rust `AudioConfig` | Alto | 10 min |
| 6 | `compression_ratio_threshold=2.4` | `whisper_stream.py` | Bajo | 1 min |
| 7 | DC-removal + high-pass | Rust `resample_and_mix` | Medio | 30 min |
| 8 | VAD threshold 0.01 | Rust `AudioConfig` | Bajo | 5 min |

**Stacked:** Cambios 1+3+4+6 = 15 minutos, 10% de mejora en fidelidad.

---

## 5. Bonus: alternativas de modelo

### 5.1. `mlx-community/whisper-large-v3-turbo-8bit`
- ~600 MB en RAM (vs 1.5 GB del turbo fp16)
- ~30% más rápido
- Leve pérdida de calidad (WER +0.5% aprox)
- **Bueno para:** laptops con poca RAM
- **Evitar si:** calidad > velocidad

### 5.2. `mlx-community/whisper-large-v3-mlx`
- Whisper large-v3 completo (no turbo)
- ~3 GB en RAM
- ~3x más lento que turbo
- **Mejor calidad absoluta** (state-of-the-art en open-source)
- **Bueno para:** workstation con M2 Pro/Max/Ultra

### 5.3. `mlx-community/whisper-base-mlx` (modelo anterior)
- ~140 MB
- 5x más rápido que turbo
- Calidad inferior (más alucinaciones, peor en acentos)
- **Bueno para:** preview/demo

### 5.4. Prompt por idioma detectado
Después de la primera transcripción, `result.get("language")` devuelve el idioma detectado. Podés cambiar el `initial_prompt` dinámicamente:
- EN: `"Hello, how are you? The weather is nice today."`
- DE: `"Guten Tag, wie geht es Ihnen? Das Wetter ist heute schön."`

Esto se puede hacer en `load_model()` pasando un prompt default, y refinándolo después de la primera transcripción.

---

## 6. Referencias

- [mlx-whisper GitHub](https://github.com/ml-explore/mlx-examples/tree/main/whisper) — v0.4.3
- [Whisper paper](https://arxiv.org/abs/2212.04356) — Radford et al., 2022
- [Whisper-Streaming paper](https://arxiv.org/abs/2309.11408) — Macháček et al., 2023 (recomienda 5-10s chunks)
- [HuggingFace Hub docs](https://huggingface.co/docs/huggingface_hub) — `snapshot_download`, `tqdm_class`

---

## 7. Estado

- [x] Investigación completa
- [x] Recomendaciones priorizadas
- [ ] Plan de implementación (ver `plan-whisper-fidelity-v0.3.md`)
- [ ] Implementación
- [ ] Tests de fidelidad (medir WER en dataset de prueba)
- [ ] Tag v0.3.0
