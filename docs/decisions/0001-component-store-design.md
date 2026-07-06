# ADR-0001: SubtitleOverlay reads directly from Zustand store

**Fecha:** 2026-07-06

**Contexto:**
El spec (§7 v0.1 tarea 5) especifica que `SubtitleOverlay.tsx` reciba `text: string` como prop. Durante la implementación surgió la disyuntiva de pasar la prop desde `App.tsx` (extrayéndola del store) o que el componente lea directamente de `useSubtitleStore()`.

**Decisión:**
`SubtitleOverlay.tsx` lee directamente del store Zustand (`useSubtitleStore`, `useSettingsStore`) en lugar de recibir `text` como prop.

**Consecuencias:**
- Positivas: elimina prop-drilling, el componente maneja su propio contrato de datos, más fácil de renderizar condicionalmente según estado del store.
- Negativas: el componente deja de ser puramente presentacional, acopla la UI al store directamente. Para v0.1 esto no es problema porque solo hay un overlay; si en el futuro hay múltiples vistas/ventanas, se puede refactorizar a prop-driven.

**Alternativas consideradas:**
1. Prop-driven como indica el spec: `App.tsx` extrae del store y pasa `text` como prop. Se descartó por añadir un nivel de indirección innecesario en una app single-screen.

**Estado:** Aceptado.
