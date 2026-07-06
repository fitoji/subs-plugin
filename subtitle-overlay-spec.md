# Subtitle Overlay — Especificación Técnica Completa (PRD + Arquitectura)

**Versión del documento:** 1.0
**Fecha:** Julio 2026
**Estado:** Aprobado para implementación
**Audiencia:** Agentes de IA de codificación (Claude Code, Codex, Gemini CLI) y desarrolladores humanos

---

## Cómo usar este documento

Este documento es la **fuente única de verdad** del proyecto Subtitle Overlay. Está escrito para que un agente de IA pueda implementar el proyecto de principio a fin sin necesidad de hacer preguntas de aclaración. Contiene:

- Objetivos, alcance y no-alcance de cada versión.
- Arquitectura de software completa (frontend, backend, pipeline de audio/IA).
- Estructura de carpetas exacta.
- Convenciones de código obligatorias.
- Tareas desglosadas por versión, con criterios de aceptación verificables.
- Prompts de arranque listos para copiar/pegar a un agente de codificación.
- Checklists de desarrollo y de QA.
- Roadmap hasta v1.0.

**Regla de oro para el agente:** si una decisión no está en este documento, el agente debe elegir la opción más simple, consistente con las convenciones ya establecidas aquí, y documentar la decisión en `docs/decisions/` en formato ADR (Architecture Decision Record) en vez de detener el trabajo para preguntar.

---

## 1. Visión del producto

### 1.1 Problema

Ver contenido audiovisual en un idioma extranjero (streaming, videollamadas, podcasts, vídeos locales) obliga a depender de subtítulos nativos de cada aplicación, si es que existen. No hay una solución universal, ligera y nativa de macOS que:

- Capture el audio del sistema (no del micrófono).
- Genere subtítulos en tiempo real con IA local (sin depender de servicios en la nube, preservando privacidad y funcionando offline).
- Se superponga a **cualquier** aplicación, sin integración específica.
- Permita traducción y aprendizaje de vocabulario sobre la marcha.

### 1.2 Solución

Una aplicación de escritorio para macOS construida con **Tauri v2**, que:

1. Muestra una ventana **overlay transparente, siempre visible** (always-on-top), sin decoraciones, flotando sobre el resto de aplicaciones.
2. Captura el audio de salida del sistema mediante ScreenCaptureKit/CoreAudio.
3. Transcribe ese audio en streaming usando Whisper (vía MLX, aprovechando Apple Silicon).
4. Traduce el texto transcrito con un LLM local o remoto.
5. Ofrece un diccionario interactivo con palabras clicables.
6. Permite guardar vocabulario para repaso posterior.

### 1.3 Usuarios objetivo

- Estudiantes de idiomas que consumen contenido audiovisual auténtico (YouTube, Netflix, podcasts).
- Usuarios de macOS con Apple Silicon (M1 o superior) que priorizan privacidad y procesamiento local.
- Público inicial: el propio desarrollador (dogfooding), con posible distribución posterior.

### 1.4 Principios de diseño

- **Local-first**: todo el pipeline de IA (STT, traducción, diccionario) debe poder ejecutarse 100% en el dispositivo, sin llamadas de red obligatorias.
- **Agnóstico de la fuente**: el overlay nunca debe integrarse con una app concreta (VLC, Netflix...); solo consume audio del sistema y expone eventos de subtítulo genéricos.
- **Bajo consumo de recursos**: la ventana overlay debe ser ligera; el pipeline de IA corre en procesos/hilos separados del hilo de UI.
- **Interfaz mínima**: la UI no debe distraer del contenido que se está viendo. Prioridad total a la legibilidad del texto.
- **Extensible sin romper contratos**: cualquier fuente de subtítulos (demo, Whisper, futura fuente) debe implementar la misma interfaz `SubtitleEvent`.

---

## 2. Alcance del proyecto

### 2.1 Dentro del alcance (hasta v1.0)

- Ventana overlay transparente, arrastrable, siempre encima.
- Captura de audio del sistema en macOS.
- Transcripción en streaming con Whisper MLX.
- Traducción de subtítulos con LLM (doble subtítulo: original + traducido).
- Diccionario interactivo con palabras clicables y definiciones.
- Guardado de vocabulario aprendido (persistencia local).
- Atajos de teclado globales (mostrar/ocultar, salir).
- Empaquetado como `.app` de macOS.

### 2.2 Fuera del alcance (explícitamente no se implementa)

- Soporte para Windows o Linux (se prioriza macOS/Apple Silicon; se puede reevaluar después de v1.0).
- Integración específica con VLC, Netflix, YouTube, etc. (el overlay es agnóstico; "compatibilidad" en el roadmap significa "funciona igual de bien viendo estas apps", no integración vía API).
- Sincronización en la nube o cuentas de usuario.
- Traducción de audio a voz (doblaje).
- Reconocimiento de idioma automático multi-idioma simultáneo (se soporta un idioma origen configurado a la vez).
- Monetización, telemetría o analítica de uso.

### 2.3 Restricción de plataforma

El proyecto asume **macOS 13+ con Apple Silicon** como plataforma primaria de desarrollo y ejecución, por la dependencia de ScreenCaptureKit y MLX. El agente no debe intentar portabilidad cross-platform salvo que se indique lo contrario en una versión futura del documento.

---

## 3. Arquitectura del sistema

### 3.1 Diagrama de flujo end-to-end (a partir de v0.2+)

```text
                     ┌─────────────────────────┐
                     │   Audio del sistema      │
                     │ (salida, no micrófono)   │
                     └────────────┬─────────────┘
                                  │
                                  ▼
                 ┌─────────────────────────────────┐
                 │ ScreenCaptureKit / CoreAudio     │
                  │ (captura vía módulo Rust)         │
                 └────────────┬─────────────────────┘
                              │  PCM 16kHz mono
                              ▼
                 ┌─────────────────────────────────┐
                 │ Buffer de audio (ring buffer)    │
                 │ ventanas de ~1-3s con solapamiento│
                 └────────────┬─────────────────────┘
                              ▼
                 ┌─────────────────────────────────┐
                 │ Whisper MLX (streaming)          │
                 │ proceso Python/MLX independiente │
                 └────────────┬─────────────────────┘
                              │ texto parcial / final
                              ▼
              ┌───────────────┴────────────────┐
              ▼                                ▼
   ┌─────────────────────┐          ┌─────────────────────┐
   │ Traductor LLM        │          │ Diccionario          │
   │ (local u online, con  │          │ (lookup on-demand,   │
   │ interfaz intercambiable)│        │ cache local)         │
   └──────────┬───────────┘          └──────────┬──────────┘
              └───────────────┬──────────────────┘
                              ▼
                 ┌─────────────────────────────────┐
                 │ Backend Rust (Tauri core)        │
                 │ orquesta procesos + estado global │
                 └────────────┬─────────────────────┘
                              │ IPC (Tauri commands + eventos)
                              ▼
                 ┌─────────────────────────────────┐
                 │ Frontend React + TypeScript      │
                 │ Ventana overlay transparente     │
                 └─────────────────────────────────┘
```

### 3.2 Componentes principales

#### 3.2.1 Frontend (React + Tauri WebView)

- **Responsabilidad**: renderizar el overlay con el texto de subtítulos actual, mostrar diccionario y controles mínimos. Las animaciones se evaluarán en fases posteriores cuando la UI esté pulida.
- **No** contiene lógica de captura de audio ni de IA. Solo consume eventos `SubtitleEvent` (y eventos derivados de traducción/diccionario) emitidos por el backend vía el sistema de eventos de Tauri.
- Gestiona su propio estado de UI (posición, visibilidad, tamaño de fuente) con Zustand, persistido localmente.

#### 3.2.2 Backend Rust (Tauri core)

- **Responsabilidad**: gestión de ventana (transparencia, always-on-top, arrastre, atajos globales), orquestación de los procesos de IA, exposición de comandos Tauri (`invoke`) y emisión de eventos hacia el frontend.
- Lanza y supervisa los procesos externos (Whisper MLX, captura de audio) como *sidecars* o procesos hijos, comunicándose vía stdio (JSON por línea) o un socket local.
- Mantiene el **contrato `SubtitleEvent`** como la única forma en que datos de subtítulos llegan al frontend, sea cual sea la fuente (demo, Whisper, futuras fuentes).

#### 3.2.3 Pipeline de audio y IA (procesos externos, no-Rust, no-JS)

- **Captura de audio**: módulo Rust con bindings a ScreenCaptureKit/CoreAudio que expone un stream PCM 16kHz mono.
- **Whisper MLX**: proceso Python que consume el stream de audio y produce transcripciones parciales/finales en streaming.
- **Traductor LLM**: módulo intercambiable detrás de una interfaz común (`Translator`), pudiendo ser un modelo local (MLX) o una API remota configurable.
- **Diccionario**: servicio de lookup de palabras con caché local en SQLite, consultado bajo demanda al hacer clic en una palabra.

### 3.3 Contrato de datos: `SubtitleEvent`

Esta interfaz es el contrato central del sistema y **no debe cambiar** entre versiones sin una migración explícita documentada en un ADR.

```ts
export interface SubtitleEvent {
  id: number
  text: string
  isFinal: boolean
  timestamp: number
}
```

Extensiones previstas para versiones futuras (v0.3+), añadidas de forma **aditiva** (nunca rompiendo el contrato base):

```ts
export interface TranslatedSubtitleEvent extends SubtitleEvent {
  translatedText: string
  sourceLanguage: string
  targetLanguage: string
}

export interface DictionaryLookupResult {
  word: string
  definition: string
  partOfSpeech?: string
  examples?: string[]
}

Eventos del sistema (adicionales al flujo de subtítulos, emitidos en canal separado):

```ts
export type SystemEvent =
  | { type: 'stt_status'; status: 'listening' | 'processing' | 'error'; message?: string }
  | { type: 'translator_status'; status: 'ready' | 'error'; message?: string }
  | { type: 'audio_status'; status: 'active' | 'silence' | 'error'; message?: string }
```
```

### 3.4 Comunicación interna

- **Frontend ↔ Backend Rust**: comandos Tauri (`invoke`) para acciones (mostrar/ocultar, cambiar configuración, solicitar lookup de diccionario) y eventos Tauri (`listen`/`emit`) para el flujo de subtítulos en tiempo real.
- **Backend Rust ↔ Procesos de IA**: stdio con mensajes JSON delimitados por línea (JSON Lines), o un socket Unix local si el volumen de datos lo requiere. Se elige JSON Lines por simplicidad salvo que el rendimiento demuestre lo contrario.
- **Persistencia local**: SQLite (vocabulario guardado, caché de diccionario) gestionada desde Rust vía `rusqlite` o `sqlx`.

### 3.5 Gestión de estado en frontend

- **Zustand** como store principal (`subtitleStore.ts`), con slices para:
  - `subtitles`: cola de eventos de subtítulos actuales (original + traducido).
  - `settings`: fontSize, opacity, backgroundBlur, maxWidth, posición de ventana.
  - `dictionary`: palabra seleccionada, resultado de lookup, estado de carga.
- El ruteo se manejará con estado simple de React en fases tempranas. Si en el futuro se necesita navegación compleja (pantalla de configuración, historial de vocabulario), se evaluará migrar a Next.js o añadir un router ligero.

---

## 4. Stack tecnológico

| Capa | Tecnología | Motivo |
|---|---|---|
| App shell | Tauri v2 | Apps nativas ligeras, acceso a APIs de sistema, tamaño reducido |
| UI | React 19 | Componentes declarativos, ecosistema maduro |
| Lenguaje frontend | TypeScript | Tipado estático, contrato `SubtitleEvent` fuerte |
| Build frontend | Vite | Rápido, integración nativa con Tauri |
| Estilos | TailwindCSS 4 | Utilidades CSS, config CSS-first (`@theme` en `globals.css`, sin `tailwind.config.ts`) |
| Estado | Zustand | Ligero, sin boilerplate |
| Animaciones | (futuro) | Sin animaciones en fases iniciales; se añadirán cuando la UI esté pulida |
| Backend | Rust | Rendimiento, acceso a APIs nativas de macOS |
| Captura de audio | ScreenCaptureKit / CoreAudio | APIs nativas de macOS para audio de sistema |
| STT | Whisper (MLX) | Transcripción local acelerada en Apple Silicon |
| Traducción | LLM (local MLX o API remota configurable) | Flexibilidad, calidad |
| Persistencia | SQLite (rusqlite/sqlx) | Ligero, embebido, sin servidor |
| Gestor de paquetes | pnpm | Rápido, eficiente en disco |

---

## 5. Estructura de carpetas

```text
subtitle-overlay/
├── src/                          # Frontend React
│   ├── components/
│   │   ├── SubtitleOverlay.tsx
│   │   ├── DictionaryPopup.tsx   # v0.4+
│   │   └── ...
│   ├── hooks/
│   │   ├── useSubtitleStream.ts
│   │   └── useDraggableWindow.ts
│   ├── store/
│   │   ├── subtitleStore.ts
│   │   ├── settingsStore.ts
│   │   └── dictionaryStore.ts    # v0.4+
│   ├── types/
│   │   └── subtitle.ts           # Interfaces SubtitleEvent y derivadas
│   ├── styles/
│   │   └── globals.css           # Tailwind v4 @theme config
│   ├── App.tsx
│   └── main.tsx
│
├── src-tauri/                    # Backend Rust
│   ├── src/
│   │   ├── main.rs
│   │   ├── window.rs             # Config ventana: transparente, alwaysOnTop, drag
│   │   ├── shortcuts.rs          # Atajos globales
│   │   ├── commands/             # Comandos Tauri invocables desde el frontend
│   │   │   ├── mod.rs
│   │   │   ├── subtitle_commands.rs
│   │   │   └── settings_commands.rs
│   │   ├── audio/                # v0.2+: orquestación de captura de audio
│   │   │   └── mod.rs
│   │   ├── stt/                  # v0.2+: orquestación del proceso Whisper MLX
│   │   │   └── mod.rs
│   │   ├── translation/          # v0.3+: interfaz Translator + implementaciones
│   │   │   └── mod.rs
│   │   ├── dictionary/           # v0.4+: lookup + caché SQLite
│   │   │   └── mod.rs
│   │   └── db/                   # v0.5+: esquema y migraciones SQLite
│   │       └── mod.rs
│   ├── Cargo.toml
│   └── tauri.conf.json
│
├── ai-pipeline/                   # Procesos externos Python/MLX (v0.2+)
│   ├── stt/
│   │   ├── whisper_stream.py
│   │   └── requirements.txt
│   └── translation/
│       ├── translator_service.py
│       └── requirements.txt
│
├── docs/
│   ├── subtitle-overlay-spec.md   # Este documento
│   └── decisions/                 # ADRs
│
├── package.json
├── pnpm-workspace.yaml
└── README.md
```

---

## 6. Convenciones de código

### 6.1 TypeScript / React

- Componentes funcionales únicamente, con hooks. No se usan componentes de clase.
- Un componente por archivo; nombre de archivo en `PascalCase` coincidente con el nombre del componente.
- Props tipadas explícitamente con `interface Props { ... }`, nunca `any`.
- Nombres de hooks personalizados con prefijo `use` (`useSubtitleStream`, `useDraggableWindow`).
- Sin lógica de negocio dentro de componentes de presentación: la lógica va en hooks o en el store.
- Formato: Prettier con configuración por defecto (2 espacios, comillas simples, sin punto y coma final se decide una vez y se documenta en `.prettierrc`, el agente debe fijar `semi: false` y `singleQuote: true` si no hay preferencia previa).
- Linter: ESLint con reglas recomendadas de `@typescript-eslint` y `eslint-plugin-react-hooks`.

### 6.2 TailwindCSS 4

- Configuración **CSS-first** en `src/styles/globals.css` usando bloques `@theme`. **No** crear `tailwind.config.ts`.
- Utilidades de Tailwind directamente en JSX; evitar CSS custom salvo para lo que Tailwind no cubre (ej. `backdrop-filter` avanzado).

### 6.3 Rust

- Seguir `rustfmt` por defecto (`cargo fmt` antes de cada commit).
- `clippy` sin warnings antes de mergear (`cargo clippy -- -D warnings`).
- Un módulo por responsabilidad (ver estructura de carpetas §5); evitar un `main.rs` monolítico.
- Comandos Tauri (`#[tauri::command]`) agrupados por dominio en `commands/`, nunca definidos inline en `main.rs`.
- Manejo de errores con `Result<T, E>` y tipos de error propios (`thiserror`), nunca `unwrap()` en código de producción (solo permitido en tests).

### 6.4 Python (pipeline de IA)

- Python 3.11+.
- Un `requirements.txt` por subcomponente (`stt/`, `translation/`), sin dependencias compartidas implícitas.
- Comunicación con Rust vía JSON Lines por stdout; logs y errores van a stderr, nunca mezclados con stdout.
- Tipado con type hints y `mypy` en modo básico.

### 6.5 Commits y ramas

- Commits en formato Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`).
- Una rama por versión del roadmap (`v0.1-overlay-base`, `v0.2-audio-whisper`, etc.), mergeada a `main` al completar los criterios de aceptación de esa versión.

### 6.6 Decisiones no cubiertas aquí

Si el agente encuentra una decisión técnica no cubierta explícitamente (ej. una librería de testing concreta), debe:
1. Elegir la opción más estándar y ampliamente adoptada del ecosistema.
2. Registrar la decisión en `docs/decisions/NNNN-titulo.md` con formato ADR (Contexto, Decisión, Consecuencias).
3. Continuar la implementación sin bloquear el progreso.

---

## 7. Tareas por versión

**Nota sobre testing**: los tests automatizados (Vitest + React Testing Library para frontend, `cargo test` para Rust, pytest para Python) se incorporarán a partir de la finalización de v0.1. Las primeras versiones priorizan la exploración funcional; una vez estable v0.1, se escriben tests que cubren el flujo principal de cada versión ya implementada.

### v0.1 — Overlay base (sin audio ni IA)

**Objetivo**: ventana transparente, siempre encima, con subtítulos de demo animados.

Tareas:
1. Crear proyecto con `pnpm create tauri-app` (React + TypeScript + pnpm).
2. Instalar dependencias: `tailwindcss`, `@tailwindcss/vite`, `zustand`.
3. Configurar TailwindCSS 4 (CSS-first, `@theme` en `globals.css`).
4. Definir tipos en `src/types/subtitle.ts`, incluyendo la interfaz `SubtitleEvent` (ver §3.3).
5. Crear `SubtitleOverlay.tsx` que reciba `text: string` como prop y lo renderice con estilo centrado, fuente grande, fondo semitransparente con blur.
6. Crear `subtitleStore.ts` (Zustand) con la cola de `SubtitleEvent` actual.
7. Configurar la ventana en `tauri.conf.json` / `window.rs`:
   - `transparent: true`
   - `alwaysOnTop: true`
   - `decorations: false`
   - `shadow: false`
   - `resizable: false`
8. Posicionar la ventana en la parte inferior central de la pantalla por defecto.
9. Implementar arrastre de ventana (`data-tauri-drag-region` o API de Tauri para mover ventana sin decoraciones).
10. Definir constantes de estilo configurables en `settingsStore.ts`: `fontSize`, `opacity`, `backgroundBlur`, `maxWidth`.
11. Implementar modo demo: array de frases de ejemplo que se van emitiendo como `SubtitleEvent` cada 2 segundos (simulando la fuente real que llegará en v0.2+).
12. Añadir icono de la app (`.icns` para macOS).
13. Generar build `.app` con `tauri build` y verificar que se ejecuta de forma independiente.

**No implementar en v0.1**: audio, Whisper, MLX, traducción, diccionario, base de datos, pantalla de configuración.

**Criterios de aceptación v0.1**:
- [ ] La app arranca y muestra una ventana sin bordes ni barra de título, con fondo transparente/translúcido.
- [ ] La ventana permanece siempre visible por encima de cualquier otra aplicación abierta.
- [ ] El texto de demo cambia automáticamente cada 2 segundos con una transición simple (sin animación elaborada).
- [ ] La ventana se puede arrastrar con el ratón a cualquier posición de la pantalla.
- [ ] `tauri build` genera un `.app` funcional que se abre por doble clic sin necesidad de `pnpm tauri dev`.
- [ ] El texto se muestra legible sobre fondos claros y oscuros (contraste suficiente).
- [ ] La app arranca en menos de 3 segundos en un Mac con Apple Silicon.
- [ ] El consumo de memoria del proceso overlay se mantiene por debajo de 200MB en reposo (sin pipeline de audio activo).

---

### v0.2 — Captura de audio + Whisper local

**Objetivo**: sustituir el modo demo por transcripción real del audio del sistema.

Tareas:
1. Implementar módulo de captura de audio del sistema (ScreenCaptureKit en macOS 13+), exponiendo un stream PCM 16kHz mono.
2. Crear proceso Python (`ai-pipeline/stt/whisper_stream.py`) que reciba ese stream y ejecute Whisper vía MLX en modo streaming, con ventanas solapadas de 1-3 segundos.
3. Definir el protocolo de comunicación (JSON Lines) entre el proceso de captura/Rust y el proceso Whisper.
4. Implementar en Rust (`src-tauri/src/audio/` y `src-tauri/src/stt/`) el lanzamiento y supervisión de estos procesos como sidecars, con reinicio automático ante fallos.
5. Emitir los resultados de Whisper como `SubtitleEvent` (con `isFinal: false` para transcripciones parciales y `true` para finales) hacia el frontend, sustituyendo el modo demo.
6. Añadir un selector de dispositivo/fuente de audio si el sistema expone más de una salida.
7. Manejar silencios y ausencia de habla (no emitir eventos vacíos).
8. Añadir indicador visual sutil de estado (escuchando / sin audio / error) en el overlay.

**No implementar en v0.2**: traducción, diccionario, guardado de vocabulario.

**Nota sobre el modelo Whisper**: el modelo por defecto será `whisper-tiny` para priorizar velocidad en tiempo real, configurable a `small` o `medium` mediante ajuste de configuración. Los modelos se cachean en `~/.cache/huggingface/` por defecto (se puede cambiar a `~/Library/Caches/subtitle-overlay/` en el futuro si es necesario). El proceso de descarga del modelo debe ocurrir una sola vez, en el primer uso, con un indicador de progreso en el overlay.

**Criterios de aceptación v0.2**:
- [ ] Reproduciendo audio en cualquier aplicación (ej. un vídeo de YouTube en el navegador), el overlay muestra transcripciones en tiempo real del audio del sistema.
- [ ] La latencia entre el habla y la aparición del subtítulo es razonable para uso conversacional (documentar la latencia medida, sin cifra objetivo impuesta salvo que se mida y quede claramente peor que Whisper base en tiempo real).
- [ ] Si no hay audio reproduciéndose, no se muestran subtítulos vacíos ni parpadeos.
- [ ] Si el proceso de Whisper se cae, el backend lo reinicia automáticamente y el overlay lo refleja (mensaje de estado breve) sin crashear la app completa.
- [ ] El modo demo de v0.1 deja de usarse en producción pero se conserva como modo de fallback/testing detrás de una flag.

---

### v0.3 — Traducción + doble subtítulo

**Objetivo**: mostrar el subtítulo original y su traducción simultáneamente.

Tareas:
1. Definir la interfaz `Translator` en Rust (`src-tauri/src/translation/mod.rs`) con al menos un método `translate(text, source_lang, target_lang) -> Result<String>`.
2. Implementar al menos una implementación concreta (LLM local vía MLX o API remota configurable mediante variable de entorno/config).
3. Extender el contrato de datos con `TranslatedSubtitleEvent` (ver §3.3), de forma aditiva.
4. Actualizar `SubtitleOverlay.tsx` para renderizar dos líneas: original arriba, traducción debajo (o intercambiables según configuración).
5. Añadir configuración de idioma origen/destino en `settingsStore.ts`.
6. Cachear traducciones recientes en memoria para evitar retraducir el mismo texto final repetido.
7. Manejar el caso de traducción lenta: mostrar el original inmediatamente y la traducción cuando esté lista, sin bloquear el subtítulo original.

**Criterios de aceptación v0.3**:
- [ ] Cada subtítulo final muestra su traducción en un segundo idioma configurado por el usuario.
- [ ] El subtítulo original aparece sin esperar a la traducción; la traducción se añade cuando está lista.
- [ ] Cambiar el idioma de destino en la configuración afecta a las siguientes traducciones sin reiniciar la app.
- [ ] Si el traductor falla, se sigue mostrando el subtítulo original sin traducción, sin romper el flujo.

---

### v0.4 — Diccionario con palabras clicables

**Objetivo**: permitir hacer clic en cualquier palabra del subtítulo para ver su definición.

Tareas:
1. Tokenizar el texto del subtítulo en palabras clicables individualmente en el frontend (`DictionaryPopup.tsx`).
2. Definir la interfaz `DictionaryLookupResult` (ver §3.3) y el comando Tauri `lookup_word(word, language)`.
3. Implementar el servicio de lookup en Rust/backend (`src-tauri/src/dictionary/mod.rs`), con caché local en SQLite para evitar lookups repetidos.
4. Mostrar un popup flotante con la definición, parte de la oración y ejemplos si están disponibles, posicionado cerca de la palabra clicada.
5. Manejar palabras no encontradas con un mensaje claro, sin bloquear la UI.

**Criterios de aceptación v0.4**:
- [ ] Cada palabra del subtítulo (original) es clicable individualmente.
- [ ] Al hacer clic, aparece un popup con la definición en menos de lo que tardaría una lectura normal (sin cifra dura; debe sentirse instantáneo tras el primer lookup gracias a caché).
- [ ] Lookups repetidos de la misma palabra no vuelven a golpear la fuente externa/lenta, se sirven de caché local.
- [ ] El popup se cierra al hacer clic fuera o al pasar al siguiente subtítulo.

---

### v0.5 — Guardado de vocabulario

**Objetivo**: permitir guardar palabras/definiciones para repaso posterior.

Tareas:
1. Diseñar esquema SQLite para vocabulario guardado (`src-tauri/src/db/mod.rs`): palabra, definición, idioma, contexto/frase original, fecha.
2. Añadir botón "guardar" en el `DictionaryPopup.tsx`.
3. Crear pantalla de listado de vocabulario guardado como una vista React dentro de la misma app (conmutada por estado, sin router). Si en el futuro se necesita navegación compleja, se evaluará migrar a un router o a Next.js.
4. Permitir eliminar entradas guardadas.
5. Persistir la base de datos en el directorio de datos de la app de macOS (`~/Library/Application Support/subtitle-overlay/`).

**Criterios de aceptación v0.5**:
- [ ] Guardar una palabra desde el popup de diccionario la añade a una lista persistente que sobrevive a reinicios de la app.
- [ ] Existe una pantalla accesible (atajo o menú) que lista todo el vocabulario guardado con su contexto original.
- [ ] Se puede eliminar una entrada guardada desde esa pantalla.

---

### v1.0 — Compatibilidad amplia y pulido final

**Objetivo**: validar que la experiencia es sólida viendo contenido real en las fuentes objetivo, y pulir la app para uso diario.

Tareas:
1. Sesiones de prueba manuales viendo/escuchando: VLC, YouTube (navegador), Netflix (navegador), mpv, Safari, Chrome, y al menos un podcast (app Podcasts o navegador).
2. Ajustar el manejo de audio para casos límite: múltiples fuentes de audio simultáneas, cambios de volumen del sistema, pausas largas.
3. Revisar rendimiento general (CPU/memoria) durante sesiones largas (>30 min) y corregir fugas de memoria o degradación.
4. Pulido de UI: tipografía final, accesibilidad de contraste, tamaños responsivos a distintas resoluciones de pantalla.
5. Pantalla de configuración completa (idiomas, fuente, opacidad, atajos personalizables).
6. Empaquetado final firmado/notarizado si se va a distribuir fuera del propio equipo (evaluar necesidad real; si es solo para uso personal, un `.app` sin firmar es aceptable y se documenta esa decisión).
7. README completo con instrucciones de instalación, requisitos (macOS, Apple Silicon, modelos MLX necesarios) y guía de uso.

**Criterios de aceptación v1.0**:
- [ ] El overlay funciona de forma equivalente (misma calidad de transcripción/traducción) viendo cada una de las fuentes listadas en la tarea 1, sin necesitar ninguna integración específica por app.
- [ ] Ninguna fuga de memoria detectable tras 30+ minutos de uso continuo (verificar con Activity Monitor o herramienta equivalente).
- [ ] Existe una pantalla de configuración accesible desde la propia app (no solo editando archivos de config a mano).
- [ ] El README permite a una persona nueva instalar y ejecutar la app siguiendo solo esas instrucciones.

---

## 8. Prompts listos para el agente

### 8.1 Prompt de arranque de proyecto (v0.1)

```text
Estás implementando la v0.1 del proyecto "Subtitle Overlay", descrito en
docs/subtitle-overlay-spec.md. Lee las secciones 1 a 7 (especialmente
§3.3 Contrato SubtitleEvent, §5 Estructura de carpetas, §6 Convenciones
de código y §7 v0.1) antes de escribir código.

Tu tarea: completar TODAS las tareas listadas en §7 → v0.1, en orden,
creando el proyecto Tauri desde cero con pnpm create tauri-app
(React + TypeScript + pnpm), configurando TailwindCSS 4 en modo
CSS-first, y dejando una app funcional que muestre subtítulos de demo
en una ventana transparente, siempre encima, arrastrable.

No implementes nada de audio, Whisper, traducción o diccionario en
esta fase (ver "No implementar en v0.1" en §7).

Al terminar, verifica cada ítem del checklist "Criterios de aceptación
v0.1" en §7 y repórtalos uno a uno como cumplido o pendiente, con
evidencia (comando ejecutado, captura de comportamiento esperado, etc.).

Si te encuentras con una decisión no cubierta en el documento, sigue
la regla de §6.6: elige la opción estándar, documenta un ADR en
docs/decisions/, y continúa sin detenerte a preguntar.
```

### 8.2 Prompt de continuación por versión (plantilla genérica)

```text
Continúas la implementación del proyecto "Subtitle Overlay"
(docs/subtitle-overlay-spec.md). La versión anterior (vX.Y) está
completa y sus criterios de aceptación verificados.

Ahora implementa la versión vX.(Y+1) tal como se describe en §7. Antes
de empezar, relee §3 (Arquitectura) para la parte del pipeline que
corresponda a esta versión, y confirma que no vas a romper el contrato
SubtitleEvent (§3.3) ni las convenciones de código (§6).

Al terminar, verifica el checklist "Criterios de aceptación vX.(Y+1)"
y repórtalo ítem por ítem.
```

### 8.3 Prompt de auditoría de arquitectura (usar antes de cada merge a main)

```text
Audita el estado actual del repositorio "Subtitle Overlay" contra
docs/subtitle-overlay-spec.md:

1. ¿Se respeta la estructura de carpetas de §5?
2. ¿El contrato SubtitleEvent (§3.3) sigue intacto y solo se ha
   extendido de forma aditiva?
3. ¿Se cumplen las convenciones de código de §6 (rustfmt/clippy
   limpios, ESLint sin errores, sin tailwind.config.ts)?
4. ¿Existen ADRs en docs/decisions/ para cualquier decisión no
   cubierta explícitamente en este documento?

Reporta discrepancias y corrígelas antes de considerar la versión
actual cerrada.
```

---

## 9. Checklist de desarrollo (transversal, aplica a cada versión)

- [ ] `cargo fmt` y `cargo clippy -- -D warnings` sin errores.
- [ ] ESLint sin errores ni warnings en `src/`.
- [ ] La app arranca con `pnpm tauri dev` sin errores en consola.
- [ ] `tauri build` genera un `.app` que arranca de forma independiente.
- [ ] El contrato `SubtitleEvent` (§3.3) no se ha modificado de forma incompatible.
- [ ] Toda decisión no cubierta por este documento está registrada como ADR en `docs/decisions/`.
- [ ] Los criterios de aceptación de la versión en curso (§7) están todos marcados y verificados.
- [ ] No se ha añadido ninguna integración específica con una app de terceros (VLC, Netflix, etc.) que rompa el principio de "agnóstico de la fuente" (§1.4).

---

## 10. Roadmap resumen

| Versión | Foco principal | Hito clave |
|---|---|---|
| v0.1 | Overlay transparente + demo | Ventana funcional, siempre encima, arrastrable |
| v0.2 | Captura de audio + Whisper local | Transcripción real en tiempo real |
| v0.3 | Traducción | Doble subtítulo original/traducido |
| v0.4 | Diccionario | Palabras clicables con definición |
| v0.5 | Vocabulario | Guardado y listado persistente |
| v1.0 | Compatibilidad amplia + pulido | Validado en VLC, YouTube, Netflix, mpv, Safari, Chrome, podcasts |

---

## 11. Glosario

- **Overlay**: ventana transparente superpuesta sobre otras aplicaciones.
- **STT**: Speech-to-Text, transcripción de voz a texto.
- **MLX**: framework de Apple para ejecutar modelos de ML de forma eficiente en Apple Silicon.
- **Sidecar**: proceso externo lanzado y gestionado por la app Tauri (aquí, los procesos Python de STT/traducción).
- **ADR**: Architecture Decision Record, documento breve que registra una decisión técnica, su contexto y sus consecuencias.
- **Local-first**: principio de diseño donde el procesamiento ocurre en el dispositivo del usuario por defecto, sin depender de servicios remotos.

---

*Fin del documento. Cualquier cambio a este documento debe incrementar la versión indicada en la cabecera y registrar el motivo en `docs/decisions/`.*
