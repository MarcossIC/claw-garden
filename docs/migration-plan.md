# Claw Garden — Plan de migración (bitácora de fases)

Documento de decisión de producto (rev. 2, 29 ago 2026):
<https://claude.ai/code/artifact/3c87147e-0487-4993-ae99-17ff9228ffa8>

Este archivo es la **bitácora de implementación** de ese documento. Por cada fase se registra:
qué se implementó, qué verificaciones se corrieron, qué casos inesperados aparecieron,
consideraciones para las fases siguientes y lo aprendido durante la implementación.

La decisión de producto NO se rediscute acá. Si algo del documento resulta inviable al
implementar, se anota en "Casos inesperados" y se decide en conversación aparte.

## Mapa de fases

| Fase | Nombre | Estado | Criterio de salida |
|------|--------|--------|--------------------|
| 0 | Identidad, split del workspace, aislamiento de perfiles | ✅ implementada 2026-09-11 · commiteada 2026-09-13 (`d018c0c`, `57119d3`) | `clawg-core` compila sin `tauri` ✅; `cargo run` escribe en `~/.clawg-dev` y no puede abrir `~/.clawg` ✅ (por tests, no por ejecución); suite verde en el número baseline ✅ 998 + 37 + 202 |
| 1 | Núcleo: `clawg serve` + `clawg mcp` | ✅ implementada, commiteada y verificada 2026-09-13 (`4623960`, `2593849`) · regresión de handshake con Claude Code 2.1.285 corregida y re-verificada 2026-09-29 (`fa048ad`, `1dd91b5`) | Claude Code recuerda algo entre dos sesiones sin la app abierta, contra perfil dev ✅ (dos `claude -p` separados, memoria #2; re-verificado el 29-09 con 2.1.285) · suite 1.397 tests, 0 failed ✅ · sin `tauri` en `clawg-cli` ✅ |
| 2 | Release e instalador (compuerta) | ✅ implementada 2026-09-29, compuerta ejecutada y verificada 2026-09-30 con autorización de Marco (`631d761`…`b9db085`) | Claw Garden instalado, arranca contra `~/.clawg`, convive con la versión dev. Cero datos migrados hasta acá |
| 3 | Migración desde engram (6 etapas) | ⚪ pendiente | `CLAUDE.md` apunta a Claw Garden, engram desregistrado, snapshot archivado |
| 4 | Panel de control | ⚪ pendiente | Encontrar y corregir una memoria mal extraída en < 30 s |
| 5 | Motor de flujos deterministas | ⚪ diseño diferido | Por definir en su propia conversación |

## Decisiones tomadas después del documento

- **2026-09-11 — Defaults del import desde engram.** Engram no tiene `importance`, `tier`,
  `source` ni `created_by_*`. En vez de inventar defaults fijos, se captura TODO el snapshot
  de engram tal cual y se **clasifica con Haiku** (`claude-haiku-4-5`) para derivar esos campos.
  Fallback a defaults declarados si no hay API key. Se diseña en la fase 3, etapas 2 y 3.

---

## Fase 0 — Identidad, split del workspace, aislamiento de perfiles

**Inicio:** 2026-09-11 · **Cierre de implementación:** 2026-09-11 · **Branch:** `feat/migration-claw-garden` · **Base:** `main @ 38027a1`

**Puntos de commit sugeridos** al cierre de la implementación (el orden respeta que cada commit compile y pase tests).
Lo que se commiteó de verdad el 2026-09-13 fueron dos commits, no tres: ver "Cierre de git de la fase 0" en la fase 1.

1. `feat(core): add profile-aware data dir resolution (CLAWG_PROFILE, ~/.clawg vs ~/.clawg-dev)` — `paths.rs` + consolidación de rutas + rename de `CLAWG_HF_BASE_URL`.
2. `refactor: unify product identity as Claw Garden / clawg` — `tauri.conf.json`, `tauri.dev.conf.json`, `package.json`, `angular.json`, strings de branding, `app.spec.ts`.
3. `refactor: split crate into clawg-core, clawg-eval and clawg-desktop workspace` — todo lo del paso 4 más `docs/migration-plan.md`.

Como el índice de git mezcla hoy los tres pasos, la forma práctica es `git reset` (mixed) y stagear por grupo.

### Estado de partida (verificado 2026-09-11)

Nada del pivot había empezado. El branch estaba 0 commits adelante de `main`.

| Identidad | Valor al inicio | Destino |
|-----------|-----------------|---------|
| Paquete Cargo | `agent-office` / lib `agent_office_lib` | crates `clawg-core`, `clawg-desktop`, `clawg-eval` |
| `tauri.conf.json` productName | `AgentOffice` | `Claw Garden` (dev: `Claw Garden (Dev)`) |
| `tauri.conf.json` identifier | `com.agentoffice.app` | `com.clawgarden.app` (dev: `com.clawgarden.app.dev`) |
| `package.json` name | `claw-garden` | `clawg-desktop` |
| Directorio de datos | `~/.claw-garden` hardcodeado en `state.rs:135` | `~/.clawg` (release) / `~/.clawg-dev` (debug), override `CLAWG_PROFILE` |
| User-Agent de `web_fetch` | `claw-garden/1.0` | `clawg/<version>` |

Datos existentes en la máquina de desarrollo al inicio:

| Directorio | Existe | Tamaño | Contenido |
|------------|--------|--------|-----------|
| `~/.claw-garden` | sí | 464,8 MB | `models/`, `sessions/`, `agents.json`, `providers.json` |
| `~/.clawg` | no | — | — |
| `~/.clawg-dev` | no | — | — |
| `~/.engram` | sí | 36,4 MB | `engram.db` + WAL (fuente de la fase 3, sólo lectura) |

### Baseline (antes de tocar nada, corrido desde `src-tauri/`)

| Comando | Resultado |
|---------|-----------|
| `cargo test --lib` | 955 passed, 0 failed |
| `cargo test --all-features` | lib 975 · golden-mine 37 · rag-eval 202 · 0 failed |
| `cargo check` | 105 warnings (preexistentes) |
| `cargo check --all-features` | 106 warnings (preexistentes) |

LOC Rust medidas: 54.018 en total. `memory/` 25.357 · `commands/` 5.314 · `hooks/` 3.191 · `tools/` 2.710 ·
`recovery/` 1.357 · `providers/` 1.239 · `models/` 1.206 · `compaction/` 961 · `services/` 833 ·
`mcp/` 736 · `session/` 349 · `config/` 328 · bins de eval 7.710 · `lib.rs`+`state.rs`+`eval_api.rs`+`main.rs` 2.727.

### Mapa de dependencias (lo que decide el split)

1. **El core no son cuatro módulos, son siete.** `models/` re-exporta `hooks::AgentHookConfig` como campo
   de `Agent`; `models/error.rs` implementa `From<session::SessionError>`; `providers/` y `config/`
   llaman a `services::{model_meta, retry, auth}`. Los tres (`services/`, `hooks/`, `session/`) son
   libres de Tauri y sólo dependen entre sí y de `models`. **Decisión:** entran a `clawg-core` tal cual.
   Cortar `models → hooks` cambiaría la forma de `Agent`, y la fase 0 es cero cambio de comportamiento.
2. **Dependencia inversa `memory → commands`.** `memory/prompt_builder.rs` y `memory/search.rs` usan
   `commands::memory_admin::{format_*, delete_memory_core, MemoryKind}`. Hay que mover esos helpers
   a `memory/` y que `commands/memory_admin.rs` los consuma desde ahí. Movimiento puro.
3. **Cinco wrappers Tauri en `memory/`**, no seis: `extractor::run_extractor`, `miner::run_miner`,
   `maintenance::start_maintenance_worker`, `worker::start_memory_worker`, `resolver::run_resolver`.
   Todos con la misma forma: reciben `tauri::AppHandle`, hacen `app.state::<AppState>()` y spawnean
   con `tauri::async_runtime`. `episodic_writer.rs` sólo menciona el patrón en un comentario.
   **Decisión:** los cinco wrappers se mueven a `clawg-desktop`; la lógica pura queda en core y se hace `pub`.
4. **`state.rs` se queda en desktop.** `AppState::new()` llama a `tauri::async_runtime::block_on` dos veces.
5. **`eval_api.rs` ya es libre de Tauri.** Va a `clawg-core` detrás del feature `eval`; los bins van a `clawg-eval`.
6. **La ruta `.claw-garden` está duplicada en cuatro lugares sin helper común**: `state.rs`,
   `services/config_store.rs`, `services/credentials.rs`, `config/loader.rs`, más espejos en los dos
   bins de eval y en `commands/workspace.rs`. Primero se consolida en un resolver único, después se renombra.
7. Feature gates existentes: sólo `eval`. `rmcp` se usa únicamente en `mcp/client.rs` (lado cliente).

### Decisiones de diseño de la fase 0

- **Perfil por compilación, con override que sólo afloja hacia dev.** `debug_assertions` → `Dev`,
  release → `Prod`. `CLAWG_PROFILE=dev|prod` puede apuntar un binario de release a dev (ensayo de
  migración). Un binario debug con `CLAWG_PROFILE=prod` **falla al arrancar**: la garantía del documento
  es que `cargo run` sea físicamente incapaz de abrir `~/.clawg`, y eso incluye el caso "me equivoqué
  de variable". Valor inválido también falla, nunca hace fallback silencioso.
- **Flavor dev de Tauri por JSON Merge Patch.** `src-tauri/tauri.dev.conf.json` con `productName`
  `Claw Garden (Dev)` e `identifier` `com.clawgarden.app.dev`, pasado con `--config` en el script
  `tauri:dev`. Verificado en docs de Tauri v2: el CLI mergea base → plataforma → `--config`, y exporta
  `TAURI_CONFIG` para que el identifier alternativo llegue también a `tauri-build` en compilación.
  Ojo: el merge reemplaza arrays completos, así que `app.windows` se repite entero en el patch.
- **El directorio `src-tauri/` conserva su nombre.** Es convención del CLI de Tauri para encontrar
  `tauri.conf.json`; lo que cambia es el nombre del paquete Cargo (`clawg-desktop`). El documento
  nombra crates, no directorios.
- **Layout del workspace:** `Cargo.toml` raíz con `members = ["crates/clawg-core", "crates/clawg-eval", "src-tauri"]`.
  `Cargo.lock` pasa a la raíz. `target/` pasa a la raíz.

### Plan de trabajo (orden por riesgo creciente)

1. **Baseline**: suite de tests y warnings antes de tocar nada.
2. **Aislamiento de perfiles** (TDD): módulo `paths` con `Profile { Dev, Prod }`, resolución por
   `debug_assertions` + override `CLAWG_PROFILE`, y reemplazo de la ruta hardcodeada.
3. **Identidad**: renombrar las cinco identidades. Cambio mecánico.
4. **Split del workspace**: `clawg-core` sin Tauri, `clawg-desktop`, `clawg-eval`.
5. **Verificación de salida**: `clawg-core` sin `tauri` en el árbol; suite en el número baseline.

### Implementado

**Paso 2 — Aislamiento de perfiles (TDD, rojo visto antes del verde)**

- Nuevo módulo `paths.rs` (sin dependencias del crate ni de Tauri; sólo `std` + `dirs`), destinado a `clawg-core`:
  - `Profile { Dev, Prod }` con `compiled_default()` (`debug_assertions` → Dev), `resolve(raw, debug_build)`
    como núcleo puro testeable, `from_env()` como wrapper fino sobre `CLAWG_PROFILE`.
  - `ProfileError { InvalidValue, ProdForbiddenInDebugBuild }` con mensajes que nombran la variable y los valores aceptados.
  - `as_str()` (`dev`/`prod`), `dir_name()` (`.clawg-dev`/`.clawg`), `mcp_server_name()` (`clawg-dev`/`clawg`, para la fase 1).
  - `data_dir(profile)`, `data_dir_from_env()`.
  - `PROJECT_CONFIG_DIR = ".clawg"`: carpeta de configuración **por proyecto**, fija, independiente del perfil.
- Las cuatro copias de `home_dir().join(".claw-garden")` (`state.rs`, `services/config_store.rs`,
  `services/credentials.rs`, `config/loader.rs`) y los espejos en los dos bins de eval pasan por el resolver único.
- `AppState::new()` es infalible, así que una mala configuración de `CLAWG_PROFILE` es **fatal al arrancar**
  con el mensaje del error. Nunca hay fallback silencioso.
- Variable de entorno `CLAW_GARDEN_HF_BASE_URL` → `CLAWG_HF_BASE_URL`. Fixtures de test `claw-garden-test-*` → `clawg-test-*`.

**Paso 3 — Identidades no-Cargo**

| Dónde | Antes | Ahora |
|-------|-------|-------|
| `tauri.conf.json` productName / título de ventana | `AgentOffice` | `Claw Garden` |
| `tauri.conf.json` identifier | `com.agentoffice.app` | `com.clawgarden.app` |
| `tauri.conf.json` frontendDist | `../dist/claw-garden/browser` | `../dist/clawg-desktop/browser` |
| **Nuevo** `tauri.dev.conf.json` | — | `Claw Garden (Dev)` / `com.clawgarden.app.dev` (merge patch, ventana completa) |
| `package.json` name / script `tauri:dev` | `claw-garden` / `tauri dev` | `clawg-desktop` / `tauri dev --config src-tauri/tauri.dev.conf.json` |
| `angular.json` proyecto / outputPath | `claw-garden` | `clawg-desktop` / `dist/clawg-desktop` |
| User-Agent de `web_fetch` (`tools/dispatch.rs`) | `claw-garden/1.0` | `clawg/<CARGO_PKG_VERSION>` |
| OpenRouter `HTTP-Referer` / `X-Title` | `https://agentoffice.app` / `AgentOffice` | `https://clawgarden.app` / `Claw Garden` |
| System prompt (`models/prompt.rs`), OAuth (`services/oauth.rs`), `index.html`, `README.md` | `AgentOffice` / `ClawGarden` | `Claw Garden` |

Los nombres de Cargo (`agent-office`, `agent_office_lib`) se renombran en el paso 4 junto con el split.

**Paso 4 — Split del workspace de Cargo**

```
Cargo.toml                    workspace: members = [crates/clawg-core, crates/clawg-eval, src-tauri], resolver 2,
                              [workspace.package] y [workspace.dependencies] compartidas
Cargo.lock                    movido desde src-tauri/ (git mv, versiones preservadas; cargo lo reescribió sin agent-office)
crates/clawg-core/            package clawg-core · lib clawg_core · features: eval, testing
  migrations/                 V1..V8 (embed_migrations! resuelve contra CARGO_MANIFEST_DIR, así que sigue a memory/migrations.rs)
  src/{lib.rs, paths.rs, eval_api.rs, config/, hooks/, memory/ (+admin.rs), models/ (+session_error.rs), providers/, services/}
crates/clawg-eval/            package clawg-eval · bins rag-eval y golden-mine · depende de clawg-core con feature eval
src-tauri/                    package clawg-desktop · lib clawg_desktop_lib · main.rs = clawg_desktop_lib::run()
  src/{lib.rs, state.rs, memory_runtime.rs (nuevo), commands/, tools/, mcp/, recovery/, compaction/, session/}
```

- **Seam 1 — dependencia inversa `memory → commands` cortada.** `MemoryKind`, los cinco `format_*`, los
  `*_core` (`list_memories_core`, `set_importance_core`, `delete_memory_core`, `get_failure_histogram_core`,
  `get_skill_steps_trend_core`), sus tipos de fila y sus tests pasaron de `commands/memory_admin.rs` a
  `clawg-core/src/memory/admin.rs`. En desktop quedan sólo los seis `#[tauri::command]` como wrappers
  (2.073 líneas → 1.938 core + 161 desktop). **Esto es, de paso, la API del panel de control de la fase 4
  y del daemon de la fase 1: ya no depende de Tauri.**
- **Seam 2 — los cinco wrappers `AppHandle` fuera de core.** `start_memory_worker`, `run_extractor`,
  `run_miner`, `start_maintenance_worker`, `run_resolver` viven ahora en `src-tauri/src/memory_runtime.rs`
  (528 líneas) con nombres y firmas intactos; los call sites (`commands/session.rs` ×6, `lib.rs` ×2)
  sólo cambiaron de path. Las pruebas de `Send` sobre `AppHandle` se movieron con ellos.
- **Regla B — `hooks/` y `session/`.** `hooks/` (3.191 LOC) entró **entero** a core: `AgentHookConfig` y
  `HookCommand` tienen `impl` inherentes en `hooks/compiled.rs`, y Rust no permite impls inherentes fuera
  del crate que define el tipo; cortar sólo el tipo obligaba a reescribir `.compile()` como funciones
  libres, o sea un cambio de lógica. `session/` **se quedó en desktop**: `SessionError` era un enum de
  11 líneas sin impls, se movió a `clawg-core/src/models/session_error.rs` y `session/mod.rs` lo re-exporta.
- **103 ampliaciones `pub(crate)` → `pub`**, todas exigidas por el compilador (bucle sobre E0603/E0616/E0624/E0451),
  ninguna especulativa. Las más grandes: `memory/db.rs` 14, `memory/mod.rs` 11, `memory/workspace.rs` 9, `memory/maintenance.rs` 9.
- **Feature `testing` en `clawg-core`.** `MemoryStore::new_for_test()`, `SEP_WORKSPACE_TEST`,
  `serialize_f32_query_pub` y `FakeEmbedder` eran `#[cfg(test)]` en core pero los usan tests de desktop,
  y `cfg(test)` no cruza crates. Ahora son `#[cfg(any(test, feature = "testing"))]` y `clawg-desktop`
  activa `testing` sólo en `[dev-dependencies]`: ningún build de producción lo linkea.
- Dependencias por crate decididas por uso real. `rmcp` quedó sólo en desktop (`mcp/client.rs`); `thiserror`
  y `sha2` salieron de desktop porque no tenían usos. `tauri-build` sigue en `src-tauri/build.rs`.
- `/target` agregado al `.gitignore` raíz (el workspace compila en la raíz). `src-tauri/.gitignore` intacto.

### Verificaciones realizadas

| Momento | Comando | Resultado |
|---------|---------|-----------|
| Tras paso 2 | `cargo test --all-features` (desde `src-tauri/`) | lib **997** (975 + 22 de `paths`), golden-mine 37, rag-eval 202, 0 failed |
| Tras paso 2 | `rg -n "claw-garden\|CLAW_GARDEN" src-tauri/src` | 0 coincidencias |
| Tras paso 3 | `cargo test --all-features` | lib 975 (corrido antes de mergear paso 2), 37, 202, 0 failed |
| Tras paso 3 | `bun run test -- --watch=false` (Vitest vía builder de Angular) | 4 archivos, **24 tests** verdes |
| Tras paso 3 | `bunx ng config projects.clawg-desktop.architect.build.options.outputPath` | `dist/clawg-desktop`; el proyecto viejo ya no existe |
| Tras corrección de `PROJECT_CONFIG_DIR` | `cargo test --lib -- paths:: config::` | 29 passed, 0 failed |
| Pasos 2 y 3 | `cargo check --all-features` warnings | 106 = baseline, no creció |
| **Salida fase 0** (desde la raíz) | `cargo tree -p clawg-core --features eval -e normal,build -i tauri` | `package ID specification 'tauri' did not match any packages` → **Tauri no está en el árbol de core** |
| Salida fase 0 | `cargo tree -p clawg-eval -e normal,build \| rg -c tauri` | 0 |
| Salida fase 0 | `cargo test --workspace --all-features` | clawg-core **838** + clawg-desktop **160** = **998** (= baseline 975 + 22 `paths` + 1 `PROJECT_CONFIG_DIR`), golden-mine 37, rag-eval 202, **0 failed** |
| Salida fase 0 | `cargo check --workspace --all-features` warnings | **55** (baseline 106; ver caso inesperado 8) |
| Salida fase 0 | `rg -ni "tauri" crates/` | 0 coincidencias, incluidos comentarios y Cargo.toml |
| Salida fase 0 | sweep `agentoffice\|agent-office\|agent_office\|claw-garden\|CLAW_GARDEN` | sólo `package-lock.json` (lockfile de npm, se regenera en el próximo install; el proyecto usa bun) |
| Salida fase 0 | `bunx tauri info` | detecta la app: frontendDist `../dist/clawg-desktop/browser`, devUrl `http://localhost:4200/`, framework Angular |

Las verificaciones de salida las corrió el agente que hizo el split **y se repitieron de forma independiente**
desde la raíz del repo antes de dar la fase por cerrada. Los números coinciden.

**Lo que NO se verificó** (por regla de trabajo: no se hacen builds de producción ni se arranca la app):
`tauri dev` con el flavor dev, `tauri build`, `ng build`. El criterio "un `cargo run` escribe en
`~/.clawg-dev`" está cubierto por los tests unitarios de `paths` y por inspección de `AppState::new()`,
no por una ejecución real de la app. Queda como primera verificación manual de la fase 1.

### Casos inesperados

1. **La carpeta de configuración por proyecto había quedado dependiente del perfil.** El primer
   intento hizo que `<project>/.claw-garden/` se resolviera con `Profile::dir_name()`, o sea
   `<project>/.clawg-dev/` en debug y `<project>/.clawg/` en release. Eso es incorrecto: el
   aislamiento dev/prod protege el directorio de datos del HOME; una carpeta que vive junto al
   proyecto la comparte quien abra ese proyecto, sin importar el build. Se corrigió a la constante
   `PROJECT_CONFIG_DIR = ".clawg"` con test propio. Nota: `ConfigLoader::{new, discover, load}` hoy
   no tienen call sites de producción; sólo `load_workspace_config_for` y `save_workspace_config`
   están cableados vía `commands/workspace.rs`.
2. **Fallback `/tmp/claw-garden` en `commands/chat.rs:696`.** Ruta literal Unix usada sólo si el
   mutex del session manager está envenenado. En Windows era una ruta inválida desde siempre.
   Ahora usa `std::env::temp_dir().join(Profile::compiled_default().dir_name())`.
3. **Test de Angular roto de antes.** `app.spec.ts` esperaba un `h1` con `Hello, claw-garden`, pero
   `app.html` no tiene ningún `h1` (renderiza `app-game-canvas`, overlay de chat, sidebar y modal).
   El test estaba stale independientemente del rename; se corrigió para verificar lo que realmente se renderiza.
4. **`~/.claw-garden` (464,8 MB) queda huérfano.** A partir de ahora un `cargo run` escribe en
   `~/.clawg-dev`, que no existe. La app va a redescargar el modelo ONNX (~470 MB) y a arrancar sin
   `providers.json`, `agents.json` ni sesiones. **Decisión pendiente de Marco**, porque es un cambio
   fuera del repo: renombrar el directorio (`mv ~/.claw-garden ~/.clawg-dev`) conserva todo; no hacerlo
   equivale a empezar limpio en dev. No se implementa ningún fallback al nombre viejo en código: eso
   sería reintroducir la ruta hardcodeada por la ventana.
5. **Herramientas de shell.** `bat`, `fd`, `eza` y `sd` no están instalados en esta máquina (ni en Git
   Bash ni en PowerShell). Se usó `rg` para todas las búsquedas.
6. **`migrations/` tuvo que mudarse a `crates/clawg-core/`.** El documento asumía que podía quedarse en
   `src-tauri/`. `refinery::embed_migrations!("migrations")` resuelve contra `CARGO_MANIFEST_DIR` del
   crate que lo invoca, y ese crate ahora es `clawg-core`.
7. **Tres tests de `rag-eval` fallaron una vez y se corrigieron.** `cli.rs::default_golden_path`,
   `cli.rs::default_out_dir` y `diff.rs::reference_results_path` construyen `CARGO_MANIFEST_DIR/../docs/eval/...`;
   el manifest quedó un nivel más profundo (`crates/clawg-eval/`) y necesitaban un `..` más. Aritmética de rutas, no lógica.
8. **Los warnings bajaron de 106 a 55 y hay que explicar por qué, no festejarlo.** Se re-midió el baseline
   en un `git worktree` de `HEAD` con target aislado: 104 diagnósticos distintos, 59 en los módulos que
   fueron a core (49 dead-code + 10 unused-import) y 45 en desktop/eval. Después del split, desktop/eval
   sigue en 45 exactos y core bajó a 7. La diferencia es un artefacto del límite de crate: un ítem `pub`
   en una lib es alcanzable por definición, así que `dead_code` deja de disparar sobre los 103 ítems
   ampliados, y los `pub use` de `hooks/mod.rs`, `memory/mod.rs` y `config/mod.rs` dejan de ser "unused".
   No se agregó ningún `#[allow]`. Los 49 dead-code de core siguen siendo código muerto de verdad, ahora
   invisible para el linter: candidato a limpieza en la fase 1, cuando se sepa qué consume el daemon.
9. **`hooks/` está en core aunque el documento lo lista como runtime congelado.** No es una decisión
   de producto, es un accidente de acoplamiento (`models::Agent` lo embebe con impls inherentes).
   Extraerlo a su propio crate (o mover sólo los tipos de configuración a `models`) es trabajo de la
   fase 1 si el daemon no lo necesita. Ver "Consideraciones".
10. **Imports muertos eliminados como efecto del seam 2.** `aux_job_enabled`/`AuxJob` en `extractor.rs`,
    `miner.rs` y `resolver.rs` sólo los usaban los wrappers que se fueron; ahora viven en `memory_runtime.rs`.
11. **Estado de git al cierre: mezcla de staged y unstaged.** Los `git mv` del split dejaron 123 cambios
    en el índice (renames detectados como `R`); los 6 archivos del renombrado de identidades y los 2 nuevos
    (`docs/migration-plan.md`, `src-tauri/tauri.dev.conf.json`) están sin stagear. **Nada commiteado.**
    `src-tauri/target/` quedó como resto del build previo al split; está ignorado y se puede borrar.

### Consideraciones para las fases siguientes

- **Fase 1 (`clawg serve` + `clawg mcp`).** El crate `clawg-cli` que produce el binario `clawg` todavía
  no existe: se crea en `crates/clawg-cli/` como cuarto miembro del workspace. El daemon necesita
  reconstruir lo que hoy hace `lib.rs` en el `setup` de Tauri y `memory_runtime.rs`: crear `AppState`
  (que sigue llamando a `tauri::async_runtime::block_on` y por eso quedó en desktop) y arrancar los cuatro
  workers. **El primer trabajo de la fase 1 es un `AppState` sin Tauri en core**, con los wrappers de
  `memory_runtime.rs` como referencia de qué exactamente arranca cada worker. `Profile::mcp_server_name()`
  ya devuelve `clawg-dev` / `clawg` para el registro en Claude Code.
- **`memory::admin` ya es la API del daemon y del panel.** Los seis `*_core` toman `&MemoryStore` y
  devuelven tipos serializables. Exponerlos por HTTP en el daemon es un adaptador, no una reescritura.
- **Feature `eval` y `testing` en core.** El daemon no debe activar ninguna de las dos. `clawg-cli`
  depende de `clawg-core` sin features.
- **`hooks/` en core es deuda declarada.** Si `clawg serve` no ejecuta hooks (y por decisión de producto
  no ejecuta tareas de coding), en la fase 1 conviene decidir: (a) mover `AgentHookConfig`/`HookCommand` y
  sus impls a `models` y sacar el runner a desktop, o (b) crear `clawg-agent-legacy` con `hooks`, `tools`,
  `mcp` cliente, `compaction`, `recovery` y `session`. La opción (b) deja el runtime congelado en un solo
  lugar con nombre explícito.
- **Datos de desarrollo.** Antes de correr la app en la fase 1, decidir sobre `~/.claw-garden`
  (renombrar a `~/.clawg-dev` o empezar limpio). Si se empieza limpio, la primera corrida descarga ~470 MB de modelo.
- **Fase 2 (instalador).** `bundle.targets` sigue en `"all"`. El flavor dev ya existe (`tauri.dev.conf.json`);
  el flavor prod es el `tauri.conf.json` base. La versión está centralizada en `[workspace.package]`
  (`0.1.0`) pero `tauri.conf.json` y `package.json` tienen la suya propia: unificar el origen de la versión
  antes del primer release. `license` y `repository` en `[workspace.package]` están vacíos.
- **Fase 3 (migración).** `clawg migrate` va en `clawg-cli`. El clasificador con Haiku para
  `importance`/`tier`/`source` (decisión del 2026-09-11) necesita un provider Anthropic: `providers/anthropic.rs`
  ya está en core.
- **Fase 4 (panel).** Los `#[tauri::command]` de `commands/memory_admin.rs` pasan a ser proxies HTTP al daemon;
  la capa de servicios Angular no cambia su contrato `invoke()`.

### Cosas aprendidas

- **El mapa de dependencias vale más que el plan.** El documento decía "cuatro módulos y seis archivos con
  Tauri"; la realidad fue siete módulos, cinco wrappers, una dependencia inversa `memory → commands` y dos
  acoplamientos vía `models`. Ninguno era visible sin leer los `use crate::` uno por uno. Explorar antes
  de cortar ahorró al menos una iteración fallida del split.
- **`cfg(test)` no cruza crates.** Cualquier helper de test que un crate consumidor necesite tiene que
  exponerse detrás de un feature (`testing`) activado sólo desde `dev-dependencies`. Es el patrón estándar
  y hay que aplicarlo desde el primer día del split, no descubrirlo a mitad de camino.
- **Los impls inherentes fijan el crate del tipo.** Un tipo de datos "puro" deja de ser movible barato
  apenas tiene un `impl Tipo { fn ... }` en otro archivo del mismo módulo. Antes de decidir "muevo sólo el
  tipo", buscar `impl NombreDelTipo` en todo el módulo.
- **Las macros que embeben archivos siguen al manifest.** `embed_migrations!`, `include_str!`,
  `include_bytes!` resuelven contra `CARGO_MANIFEST_DIR`. Al mover un módulo de crate, mover también lo que embebe.
- **Menos warnings no siempre es mejor código.** Hacer `pub` para cruzar el límite de crate apaga
  `dead_code` sobre código que sigue muerto. Cuando baja el conteo, medir el baseline en un worktree
  limpio y atribuir la diferencia antes de aceptarla.
- **El aislamiento por perfil tiene un alcance preciso.** Protege el directorio de datos del usuario;
  no se propaga a carpetas que viven junto a un proyecto. Confundir los dos hubiera hecho que un mismo
  repo tuviera dos configuraciones según quién lo abre con qué build.
- **JSON Merge Patch reemplaza arrays enteros.** En el flavor dev de Tauri hay que repetir el objeto
  `windows[0]` completo, no sólo el `title`.
- **Un test que ya estaba roto se descubre cuando alguien por fin corre la suite.** `app.spec.ts`
  llevaba tiempo esperando un `h1` inexistente. La suite de frontend no formaba parte de ningún gate.

---

## Fase 1 — Núcleo: `clawg serve` + `clawg mcp`

**Inicio:** 2026-09-13 · **Branch:** `feat/migration-claw-garden` · **Base:** `57119d3` (cierre de fase 0)

### Cierre de git de la fase 0 (2026-09-13)

La bitácora sugería tres commits (perfiles → identidad → split). Se hicieron **dos**: `d018c0c refactor: unify
product identity as Claw Garden / clawg` (sólo archivos no-Cargo: `tauri.conf.json`, `tauri.dev.conf.json`,
`package.json`, `angular.json`, `index.html`, `README.md`, `app.spec.ts`) y `57119d3 refactor: split crate into
clawg-core, clawg-eval and clawg-desktop workspace` (todo el Rust, `paths.rs`, `.gitignore`, esta bitácora).
Motivo: `paths.rs` nació en `src-tauri/src/` y se mudó a `crates/clawg-core/` dentro del mismo trabajo; reconstruir
un estado intermedio que compilara sólo para separar "perfiles" de "split" no aportaba nada verificable. El commit de
identidad sí es independiente: no toca una línea de Rust. `~/.claw-garden` sigue sin decisión (caso inesperado 4 de la fase 0).

### Estado de partida (verificado 2026-09-13 con exploración de la API pública de `clawg-core`)

Lo que el documento de producto asumía y lo que hay de verdad:

| Supuesto del documento | Realidad | Consecuencia |
|------------------------|----------|--------------|
| "El servidor MCP sólo falta exponerlo; la dependencia y el transporte ya están" | `rmcp 1.3.0` está sólo con features `client` + `transport-child-process` (`mcp/client.rs`, lado cliente). Sin `server` ni `transport-io` | Es agregar features, no reescribir. Pero no hay ni una línea de servidor MCP para reusar |
| El daemon "toma el `AppState` y los cuatro workers" | `AppState` mezcla memoria con runtime de agente (providers, agents, sesiones, tools, MCP pool, confirmaciones HITL). Sólo 6 campos son memoria: `base_dir`, `memory_store`, `memory_tx`, `embedder`, `tokenizer`, `access_tracker` | Se extrae un `MemoryRuntime` con esos seis campos a core; `AppState` lo compone. El daemon nunca ve providers ni agentes |
| Los workers se arrancan desde core | Los cinco wrappers de `memory_runtime.rs` son `pub(crate)` del crate desktop y spawnean con `tauri::async_runtime`. Los futures puros (`memory_worker`, `run_maintenance_loop`) sí son `pub` en core | Core expone **futures sin spawnear**; quien arranca elige el runtime (`tauri::async_runtime::spawn` en desktop, `tokio::spawn` en el daemon) |
| El bootstrap del embedder es parte de la memoria | `run_embedder_bootstrap`, `EmbedderEventSink` y las tres closures inyectables viven en `src-tauri/src/lib.rs`, aunque no dependen de Tauri (el sink es un trait; `StreamEvent::EmbedderDownload` ya está en core) | Se mueve a `clawg_core::memory::bootstrap`; desktop conserva `AppHandleSink`; el daemon usa un `LogSink` |
| Hay una API de búsqueda para exponer por MCP | `search_all_memories` y `SearchScope` son `pub(crate)`. La única puerta pública es `build_memory_prompt`, que devuelve un **string de prompt**, no hits | Se abre la visibilidad (`pub`), sin cambiar lógica. `RankedHit` ya es `pub` |
| Guardar una memoria explícita es "insertar una fila" | `MemoryEvent` es un struct plano (agent, project, role, content, source: `Chat`/`Compaction`/`CodeFile`). El único camino que chunk-ea, embebe, deduplica y persiste es `memory_worker`, consumidor único por diseño (gate de dedup) | Se agrega `MemorySource::Explicit` y overrides opcionales (`memory_type`, `importance`) al evento. El guardado explícito **entra por el mismo worker**, así conserva dedup y la garantía de consumidor único |
| La memoria ya se expone al agente como herramienta | **No existe ninguna tool de memoria** en `tools/`. La memoria entra al chat sólo por `compute_memory_prefix` (prefijo del system prompt) y sale por `push_and_persist` | Las tools MCP se diseñan desde cero: `save_memory`, `search_memory`, `recall_context`, `list_memories`, `delete_memory`, `memory_status` |
| Hay algo de HTTP/CLI para el daemon | No hay `axum`, ni `clap`, ni logger concreto (`log` es sólo la fachada; desktop usa `tauri-plugin-log`). El único `TcpListener` es el callback OAuth de un solo uso | Dependencias nuevas: `axum 0.8`, `clap 4`, `env_logger`; `rmcp` gana `server` + `transport-io` + `macros` + `schemars` |

Otros datos duros: la base se llama `memory.db` (no `clawg.db` como dice el documento; no se renombra en esta fase,
renombrar archivos de datos es trabajo de la migración). `AccessTracker::new` hace `tokio::spawn` adentro, por eso
`MemoryRuntime::open` es `async` y exige runtime. `episodic_writer` es `pub(crate)`: la única vía pública al camino
episódico es `mine_session_core`, que el daemon no necesita en esta fase (no hay sesiones de chat que minar).

### Decisiones de diseño de la fase 1

- **Topología exactamente la del documento:** `clawg serve` es el único dueño de SQLite, del embedder ONNX y de los
  workers; escucha HTTP en `127.0.0.1` con token en archivo; `clawg mcp` es un puente stdio → HTTP que, si no hay
  daemon, lo lanza desacoplado y reintenta (modelo Ollama). La UI queda para la fase 4 como cliente HTTP.
- **Un solo protocolo en el daemon: JSON sobre HTTP.** El puente MCP y, después, la app de escritorio hablan lo mismo.
  No se usa Streamable HTTP de MCP en el daemon: obligaría a la UI a hablar MCP para administrar, y el panel de la
  fase 4 necesita `list/set_importance/delete`, que son REST naturales.
- **Puertos fijos por perfil** (`Profile::default_port()`): prod `27380`, dev `27381`; `--port 0` para efímero en tests.
  El puerto real se publica en `<data_dir>/daemon.json` junto con `pid`, `version` y `profile`; el token en
  `<data_dir>/token` (0600 en Unix; en Windows el HOME ya es por usuario). El puente lee `daemon.json`, así que un
  puerto no-default también se descubre.
- **Instancia única por perfil** con lock exclusivo (`fs2`, ya en el árbol) sobre `<data_dir>/daemon.lock`,
  sostenido toda la vida del proceso. Un segundo `serve` falla con mensaje, nunca abre la base dos veces.
- **`save_memory` es asíncrono (202 queued).** Entra por `memory_tx` al worker único. No devuelve id: dar ids
  sincrónicos exigiría o bien un canal de ack en `MemoryEvent` (rompe `Clone`/`Serialize`) o bien llamar al
  `process_event` interno desde el handler HTTP, y eso rompería la garantía de consumidor único del gate de dedup.
  Se acepta para esta fase; el id se obtiene con `search_memory`/`list_memories`.
- **Identidad del cliente:** `agent_id` = `clientInfo.name` del `initialize` de MCP (Claude Code se presenta como
  tal), fallback `mcp`. `project_id` = `derive_project_id(cwd)` del puente, porque Claude Code lanza los servidores
  MCP con el cwd del proyecto; las tools aceptan `project_path` para sobreescribir.
- **`hooks/` se queda en core esta fase.** El daemon no lo usa y no le cuesta nada; mover tipos a `models` o crear
  `clawg-agent-legacy` sigue siendo deuda declarada, no bloquea el criterio de salida.
- **Nada de tareas de coding en el daemon.** Sin providers, sin agentes, sin extractor/miner/resolver en esta fase:
  esos tres requieren un `AiProvider` y una sesión de chat que el daemon no tiene. Se arrancan sólo ingest, access y
  maintenance. Extractor/miner/resolver vuelven cuando haya una fuente de sesiones (fase 3 o 4), decisión aparte.

### Plan de trabajo

1. **Paquete A — runtime headless en core** (TDD): `clawg_core::runtime::MemoryRuntime`, `memory::bootstrap`,
   `MemorySource::Explicit` + overrides, `search_all_memories` público, `Profile::default_port()`, desktop
   recompuesto sobre el runtime sin cambio de comportamiento.
2. **Paquete B — crate `clawg-cli`** (TDD): `clawg serve` (axum, lifecycle, lock, token), `DaemonClient` con
   autostart, `clawg mcp` (rmcp stdio) con las seis tools, `clawg status`.
3. **Verificación independiente** de la suite y del árbol de dependencias, más revisión del diseño de los endpoints.
4. **Registro manual en Claude Code** (lo hace Marco, es acción fuera del repo): `claude mcp add clawg-dev -- <ruta>/clawg mcp`
   y la prueba de salida: guardar algo en una sesión, cerrarla, recuperarlo en otra, con la app cerrada.

### Implementado

**Paquete A — runtime headless en core** (commit `4623960 feat(core): add headless MemoryRuntime and explicit memory ingest`)

```
crates/clawg-core/src/runtime.rs          nuevo, 480 líneas: MemoryRuntime { base_dir, store, memory_tx, embedder, tokenizer, access_tracker }
                                          open(base_dir) async · ingest_worker(rx) · maintenance_worker() · workspaces() · embedder_ready()
                                          new_for_test(base_dir) [feature testing] · ingest_worker_future(...) · maintenance_worker_future(...)
crates/clawg-core/src/memory/bootstrap.rs nuevo, ~800 líneas: EmbedderEventSink, EnsureModelFn/LoadEmbedderFn/ContractVerifyFn,
                                          run_embedder_bootstrap, progress_to_stream_event, production_deps(store), LogSink + 11 tests movidos + 3 nuevos
crates/clawg-core/src/memory/events.rs    MemorySource::Explicit · MemoryEvent { memory_type: Option<String>, importance: Option<f64> } con serde(default)
crates/clawg-core/src/memory/store_insert.rs  from_event_and_chunk honra los overrides cuando source == Explicit
crates/clawg-core/src/memory/search.rs    SearchScope, search_all_memories → pub; RankedHit deriva Serialize
crates/clawg-core/src/memory/clock.rs     TimestampMs deriva Serialize (transparent, i64 ms)
crates/clawg-core/src/memory/embedder.rs  FakeEmbedder: Default e impl Embedder ahora con el mismo cfg que el struct (any(test, eval, testing))
crates/clawg-core/src/paths.rs            Profile::default_port() prod 27380 / dev 27381 · daemon_state_file() · token_file()
crates/clawg-core/tests/public_api.rs     nuevo: 8 tests que prueban la visibilidad desde un crate externo
src-tauri/src/state.rs                    AppState::new(): un solo block_on sobre MemoryRuntime::open, destructurado a los campos existentes
src-tauri/src/lib.rs                      −900 líneas: sólo queda AppHandleSink y el wrapper start_embedder_bootstrap
src-tauri/src/memory_runtime.rs           start_memory_worker / start_maintenance_worker delegan en los futures de core
```

- **Los workers son futures, no spawns.** `ingest_worker(rx)` y `maintenance_worker()` devuelven `impl Future + Send + 'static`
  y el que llama elige el runtime: `tauri::async_runtime::spawn` en desktop, `tokio::spawn` en el daemon. Es lo único que
  permite que un mismo cuerpo sirva a los dos sin que core sepa qué runtime hay arriba.
- **El guardado explícito entra por el worker.** `POST /v1/memories` (paquete B) construye un `MemoryEvent` con
  `source: Explicit` y lo manda por `memory_tx`. Mismo chunker, mismo embedder, mismo gate de dedup que un turno de chat.
- Con `Explicit`, `memory_type` desconocido o ausente cae a `SemanticFact` (hecho declarado por el usuario; sigue siendo
  "claim-bearing", así que la resolución de contradicciones lo cubre). `created_by_type = User`.

### Verificaciones realizadas (paquete A)

| Momento | Comando | Resultado |
|---------|---------|-----------|
| Cada unidad | rojo visto antes del verde (E0599 `default_port`, E0432 `clawg_core::runtime`, E0004 `Explicit not covered` en `store_insert.rs:297`, etc.) | 7 unidades, todas rojo → verde |
| Cierre A (agente) | `cargo test --workspace --all-features` | core 878 + `public_api` 8, desktop 150, golden-mine 37, rag-eval 202, 0 failed |
| **Cierre A (independiente, desde la raíz)** | `cargo test --workspace --all-features` | **mismos números: 878 + 8 + 150 + 37 + 202 = 1.275, 0 failed** (baseline 998; desktop bajó 160 → 150 porque 11 tests del bootstrap se mudaron a core y entró 1 nuevo) |
| Cierre A | `cargo check --workspace --all-features` warnings | 55 = baseline, mismo conjunto |
| Cierre A | `cargo tree -p clawg-core --features eval -e normal,build -i tauri` | no match |
| Cierre A | `rg -n -i tauri crates/` | 0 (los comentarios nuevos dicen "desktop runtime") |

### Casos inesperados (paquete A)

1. **El dominio de `importance` es 0–10, no 0–1.** El brief pedía clamp a `[0, 1]`. Antes de obedecer, el agente
   verificó: `tier.rs:28` promueve a tier 1 con `importance >= 9.0`, los defaults existentes son 5.0/6.0/8.0 y los
   tests de `admin.rs` usan 9.0. Un clamp a `[0, 1]` habría fijado TODA memoria explícita en la importancia mínima,
   lo contrario de la intención. Se clampea a `[0, 10]`; `NaN` cae a 5.0. **Las tools MCP reciben importancia 0–10.**
2. **`memory_type` se mapea al enum, no se guarda crudo.** La columna es `TEXT` sin `CHECK`, así que un string
   arbitrario persistiría, pero `compute_tier` y el gate de contradicciones clasifican vía `MemoryType::from_db_str`
   y un valor desconocido saca la fila silenciosamente de los dos. Se mapea; desconocido → `SemanticFact`.
3. **Bug preexistente en `FakeEmbedder`.** El struct estaba gateado con `any(test, eval, testing)` pero su `Default`
   y su `impl Embedder` sólo con `any(test, eval)`: con el feature `testing` solo, el tipo existía pero era
   inconstruible. Nadie lo había notado porque desktop era el único consumidor y activa `testing` en un build de test.
4. **`RankedHit` no derivaba `Serialize`.** Para devolver hits por HTTP hizo falta agregarlo, y eso arrastró a
   `TimestampMs` (`#[serde(transparent)]`, viaja como i64 en milisegundos).
5. **La visibilidad no se puede testear desde adentro del crate.** `pub` vs `pub(crate)` sólo lo prueba un crate
   ajeno que linkee: por eso existe `crates/clawg-core/tests/public_api.rs`. Los casos que necesitan
   `new_for_test` están detrás de `feature = "testing"` para que `cargo test -p clawg-core` pelado siga compilando.
6. **`progress_to_stream_event` tuvo que mudarse con el bootstrap.** El forwarder de progreso la llama; el brief no
   la listaba porque la exploración la había pasado por alto.

**Paquete B — crate `clawg-cli`** (commit `2593849 feat(cli): add clawg binary with serve daemon and mcp stdio bridge`)

```
Cargo.toml                          members += crates/clawg-cli
crates/clawg-cli/Cargo.toml         package clawg-cli · [[bin]] clawg · [lib] clawg_cli (para tests in-process)
crates/clawg-cli/src/main.rs        103  parse + dispatch; exit 0 ok, 1 fallo (mensaje por stderr), 2 uso (clap)
crates/clawg-cli/src/cli.rs         288  clap derive: serve [--profile] [--port] · mcp [--profile] [--no-autostart] · status · stop
crates/clawg-cli/src/daemon/lifecycle.rs 353  DaemonState → daemon.json · token (32 bytes hex) · SingleInstanceLock (fs2, daemon.lock) · read/clear
crates/clawg-cli/src/daemon/api.rs  575  axum Router + AppCtx · bearer middleware (comparación en tiempo constante) · handlers
crates/clawg-cli/src/daemon/serve.rs 203 lock → MemoryRuntime::open → spawn bootstrap/ingest/maintenance → bind → publicar estado → serve → clear_state
crates/clawg-cli/src/client.rs      848  DaemonClient (reqwest) · ensure_running con autostart · DaemonSpawner (ProcessSpawner + fake de test)
crates/clawg-cli/src/mcp/server.rs  830  ClawgMcp (rmcp 1.3, #[tool_router]) · get_info · agent_id desde clientInfo.name
crates/clawg-cli/src/mcp/tools.rs   360  formateo puro de resultados · mapeo params → requests
crates/clawg-cli/src/status.rs      122  reporte JSON de `clawg status`
crates/clawg-cli/tests/api.rs       498  18 tests de integración: router real + ingest worker real sobre store en memoria
```

Dependencias nuevas (todas sin `tauri` en el grafo): `axum 0.8.9`, `clap 4.6.6`, `env_logger 0.11.11`, `schemars 1.2.1`;
`rmcp 1.3.0` gana los features `server`, `transport-io`, `macros`, `schemars` (el desktop sigue con `client` +
`transport-child-process`; cargo unifica en una sola build, los features son aditivos).

**Contrato HTTP del daemon** (`127.0.0.1:<puerto>`; todo bajo `/v1` exige `Authorization: Bearer <token>`; un solo
401 `{"error":"unauthorized"}` para cualquier forma de fallar; errores siempre `{"error":"<mensaje>"}`):

| Método | Ruta | Body | OK | Fallos |
|--------|------|------|----|--------|
| GET | `/health` (sin auth) | — | 200 `{status, version, profile, pid, embedder: "ready"\|"loading", uptime_s}` | — |
| POST | `/v1/memories` | `{content, agent_id, project_id?, memory_type?, importance?}` | 202 `{"status":"queued"}` | 400 contenido vacío · 500 worker caído |
| POST | `/v1/search` | `{query, agent_id, project_id?, limit?}` (limit se clampea a 1..=50, default 10) | 200 `{"hits":[RankedHit…]}` | 503 `embedder_not_ready` · 500 |
| POST | `/v1/context` | `{query, agent_id, project_id?}` | 200 `{"prompt": String}` (vacío es válido) | 500 |
| GET | `/v1/memories` | `?kind&tier&memory_type&limit&offset` | 200 `{"items":[MemoryAdminRow…]}` | 400 kind desconocido · 500 |
| PATCH | `/v1/memories/{kind}/{id}` | `{importance}` | 200 `{"updated": n}` | 400 · 404 si n = 0 · 500 |
| DELETE | `/v1/memories/{kind}/{id}` | — | 200 `{"deleted": n}` | 400 · 404 si n = 0 · 500 |
| POST | `/v1/shutdown` | — | 202 `{"status":"shutting_down"}` | — |

**Tools MCP de `clawg mcp`** (todas devuelven texto; los fallos van como `CallToolResult::error` con una frase
legible, nunca como error de protocolo, para que el modelo sepa qué hacer después):

| Tool | Parámetros | Devuelve |
|------|------------|----------|
| `save_memory` | `content`, `memory_type?` (uno de los 7 valores de `MemoryType`, listados en el schema), `importance?` 0–10, `project_path?` | `queued` |
| `search_memory` | `query`, `limit?` 1–50, `project_path?`, `all_projects?` | lista numerada `N. #id [Kind/type/tN score=…] contenido` (500 chars) o `no memories found` |
| `recall_context` | `query`, `project_path?` | el prompt de `build_memory_prompt` o `no context yet` |
| `list_memories` | `kind?`, `tier?`, `memory_type?`, `limit?`, `offset?` | tabla compacta `#id kind type tN imp=x acc=n status summary` |
| `delete_memory` | `id`, `kind` | `deleted n` |
| `memory_status` | — | `/health` + perfil + data dir |

`agent_id` sale de `clientInfo.name` del `initialize` (fallback `mcp`); `project_id` = `derive_project_id(cwd)` del
puente, sobreescribible con `project_path`. Los logs de `clawg mcp` van SIEMPRE a stderr (`env_logger` con
`Target::Stderr` explícito): stdout es el transporte MCP.

### Verificaciones realizadas (paquete B, repetidas de forma independiente desde la raíz)

| Comando | Resultado |
|---------|-----------|
| `cargo test --workspace --all-features` | clawg-cli **92** + `tests/api` **18**, core 878 + 8, desktop 150, golden-mine 37, rag-eval 202 → **1.385 tests, 0 failed** |
| `cargo check --workspace --all-features` warnings | **55** = baseline; el crate nuevo aporta 0 |
| `cargo clippy -p clawg-cli --all-targets` | 0 warnings del crate (los 31 que lista son de core, preexistentes) |
| `cargo tree -p clawg-cli -e normal -i tauri` | no match |
| Rojo → verde | 9 unidades con rojo visto (cli 5, lifecycle 15, api 6, integración 18, client 12, serve 4, tools 20, server 8, status 5) |
| Revisión manual de `serve.rs`, `ProcessSpawner`, `require_bearer` | lock antes de abrir la base; `daemon.json` se escribe recién con el socket vinculado y se borra en toda salida; stdin del hijo en null (no se come frames MCP); comparación de token sin early-exit |

**Lo que NO se verificó, y es el criterio de salida de la fase:** ningún test ni el agente ni yo ejecutamos el binario
real. `daemon::serve::run` abre la base real y dispara la descarga del modelo (470 MB), así que queda fuera de la
suite por diseño. El criterio "Claude Code recuerda algo entre dos sesiones sin la app abierta" es una **verificación
manual pendiente de Marco** (ver "Pendiente para cerrar la fase").

### Casos inesperados (paquete B)

1. **Bug preexistente en core, NO corregido: el tracker de accesos mezcla ids de tablas distintas.**
   `prompt_builder.rs:166` hace `tracker.record(hits.iter().map(|h| h.id).collect())` sobre hits de las TRES tablas
   (`memory`, `episodic_trajectory`, `procedural_skill`), pero el flush de `AccessTracker` (`access.rs:146`) es un
   único `UPDATE memory`. Un hit episódico con id 7 promueve la fila 7 de `memory`, que no tiene nada que ver.
   Los ids son autoincrement por tabla, así que la colisión es la norma, no la excepción. `POST /v1/search` del daemon
   registra sólo `TableKind::Memory`, o sea que el daemon NO reproduce el bug, pero `recall_context` (que usa
   `build_memory_prompt`) sí. **Recomendación:** filtrar por `kind == Memory` en `prompt_builder.rs` con un test que
   meta un hit episódico y verifique que `memory.access_count` no cambia. Es un cambio de una línea más test, pero
   toca el camino de tier del RAG y merece su propio commit con Marco al tanto.
2. **`Peer::new` es `pub(crate)` en rmcp 1.3.** No se puede construir un `RequestContext` en tests, así que los
   cuerpos de las tools delegan en funciones `*_impl` que reciben `agent_id` resuelto; el `#[tool]` sólo extrae el
   nombre del cliente del contexto y llama. Es el patrón `*_core` de `memory/admin.rs` aplicado al puente.
3. **`ServerInfo` es `#[non_exhaustive]`.** Se construye con `ServerInfo::new(caps)` y asignación de campos, no con
   literal de struct.
4. **`FakeEmbedder` no tiene geometría semántica** (es SipHash del texto). El test `search_finds_a_saved_memory`
   pasa porque la misma query reproduce exactamente el vector guardado (distancia 0): prueba el cableado, no la
   calidad del retrieval. La calidad se mide sólo con `rag-eval`, como siempre.
5. **`ensure_running(profile, …)` resuelve el `data_dir` real.** Un test que lo llamara hablaría con el daemon dev
   de la máquina. Se agregó `ensure_running_in(base_dir, …, timeout)` para testear el autostart y el timeout
   (300 ms en test en vez de 60 s).
6. **`clap`, `axum`, `env_logger`, `rmcp`, `schemars` van por crate, no en `[workspace.dependencies]`.** El manifest
   raíz reserva esa tabla para dependencias compartidas por dos o más miembros; ninguna de estas lo es.
7. **Zombi acotado en Unix.** `ProcessSpawner` suelta el hijo sin `wait()`; queda una entrada zombi mientras viva el
   puente. Es a lo sumo una por proceso (el autostart ocurre una sola vez). En Windows no aplica.
8. **Sin ACL en Windows para `token` y `daemon.json`.** En Unix se aplica 0600. En Windows se documenta como no-op:
   `%USERPROFILE%` ya excluye a otros usuarios no administradores. Si el instalador de la fase 2 cambia el data dir
   a algo compartido, esto hay que revisarlo.

### Pendiente para cerrar la fase (acciones fuera del repo, las hace Marco)

1. Decidir sobre `~/.claw-garden` (464,8 MB): `mv ~/.claw-garden ~/.clawg-dev` conserva el modelo y evita la descarga.
2. Compilar el binario: `cargo build -p clawg-cli` (debug es suficiente para dev; el binario debug rechaza `--profile prod`).
3. Registrar el puente en Claude Code para el perfil dev:
   ```
   claude mcp add clawg-dev -- "C:\Users\marco\Desktop\Marcos\mis_proyectos\claw-garden\target\debug\clawg.exe" mcp --profile dev
   ```
4. Prueba de salida: en una sesión de Claude Code, `save_memory` con algo distintivo; cerrar la sesión; en otra
   sesión, `search_memory` con una query relacionada; debe aparecer, **con la app de escritorio cerrada**.
   `clawg status --profile dev` muestra si el daemon quedó vivo y si el embedder está `ready`; el log del daemon
   arrancado por el puente está en `~/.clawg-dev/logs/serve.log`.
5. Si algo falla, primero `clawg stop --profile dev` y `clawg serve --profile dev` en una terminal para ver los logs
   en vivo.

### Consideraciones para las fases siguientes

- **Fase 2 (instalador).** El bundle tiene que incluir `clawg.exe` y ponerlo en el PATH: hoy `tauri.conf.json` sólo
  bundlea la app. `externalBin` de Tauri es la vía documentada, pero exige el sufijo target-triple en el nombre del
  binario. La versión sigue triplicada (`[workspace.package]`, `tauri.conf.json`, `package.json`); `clawg --version`
  lee la del workspace.
- **Fase 3 (migración).** `clawg migrate` entra como quinto subcomando en `cli.rs`; el import puede escribir por
  `POST /v1/memories` contra el daemon (ya soporta `memory_type` e `importance`) o directo por `MemoryRuntime` si
  el daemon no está corriendo. Ojo con `save_memory` asíncrono: un import de 2.455 filas necesita saber cuándo terminó
  el worker; hace falta o un endpoint de "cola vacía" o hacer el import por `MemoryRuntime` en proceso.
- **Fase 4 (panel).** Los seis `#[tauri::command]` de `memory_admin.rs` se vuelven proxies de `DaemonClient`
  (ya existe, con los mismos verbos). `AppState` de desktop puede dejar de abrir la base cuando eso pase; hasta
  entonces desktop y daemon **no deben correr a la vez sobre el mismo perfil**: los dos abrirían `memory.db` y
  arrancarían su propio ingest worker. El lock de `daemon.lock` sólo protege daemon contra daemon.
- **Extractor / miner / resolver** siguen sin correr en el daemon: necesitan un `AiProvider` y sesiones de chat.
  Cuando la migración traiga memorias de engram, el resolver de contradicciones es el primero que va a hacer falta.
- **`hooks/` en core**: deuda declarada intacta. El daemon no lo linkea funcionalmente pero sí lo compila.

### Cosas aprendidas (fase 1)

- **Devolver futures, no spawns, es lo que hace que un core sirva a dos runtimes.** `tauri::async_runtime::spawn` y
  `tokio::spawn` no son intercambiables (el setup hook de Tauri corre fuera de contexto tokio). Si core spawneara,
  habría que elegir uno y el otro lado se rompe.
- **Una fila `TEXT` sin `CHECK` no es una columna libre.** `memory_type` acepta cualquier string, pero todo el que
  lo lee lo parsea a un enum y descarta lo que no reconoce. La restricción vive en los lectores, no en el esquema;
  hay que buscarla ahí antes de "guardar tal cual".
- **Los dominios numéricos hay que verificarlos contra quien los consume.** El brief decía `importance` en 0–1;
  `tier.rs` dice 0–10. Obedecer el brief habría sido un bug silencioso que ningún test del paquete hubiera visto.
- **La visibilidad se testea desde afuera.** `cargo test` dentro del crate no distingue `pub` de `pub(crate)`.
  Un `tests/public_api.rs` que linkea como crate ajeno es la única prueba real.
- **Un fake determinista prueba plumbing, no calidad.** Hay que decir explícitamente qué prueba cada test de
  búsqueda para no confundir "encontró la fila" con "el retrieval funciona".
- **Con stdio como transporte, stdout es sagrado.** Loggers, `println!` de debug y el stdin de cualquier hijo
  spawneado son los tres lugares por donde se corrompe el protocolo MCP. Los tres quedaron cerrados en código,
  no por convención.

### Verificación manual del criterio de salida (2026-09-13, ejecutada a pedido de Marco)

Todo con el binario debug `target/debug/clawg.exe` (54 MB, `clawg 0.1.0`) contra el perfil dev. La app de escritorio
estuvo cerrada todo el tiempo.

**Datos.** Se copió sólo `~/.claw-garden/models/multilingual-e5-small/` (465 MB) a `~/.clawg-dev/models/`.
`~/.claw-garden` quedó intacto: la decisión de renombrarlo o borrarlo sigue siendo de Marco, pero ya no bloquea nada.
El modelo copiado pasó la verificación de checksums: el embedder estuvo `ready` a los 12 s de arrancar, sin descarga.

| Paso | Comando / acción | Resultado |
|------|------------------|-----------|
| Perfil | `clawg serve --profile prod` con binario debug | rechazado: `--profile prod is forbidden in a debug build; use --profile dev, or run a release build to use prod`, exit 1 |
| Sin daemon | `clawg status --profile dev` | `running: false`, `daemon: null`, exit 1 |
| Arranque | `clawg serve --profile dev` | crea `~/.clawg-dev/memory.db`, aplica V1..V8, publica `daemon.json` (pid, port 27381) y `token` (64 hex), `daemon.lock` |
| Health | `GET /health` | `{"status":"ok","version":"0.1.0","profile":"dev","embedder":"loading"}` → `"ready"` a los 15 s de uptime |
| Auth | `GET /v1/memories` sin token / con token inválido | 401 / 401 |
| Guardar | `POST /v1/memories` con `memory_type: semantic_fact`, `importance: 8` | 202 `queued`; la fila apareció en `GET /v1/memories` con `importance: 8.0`, `tier: 3` |
| Validación | `POST /v1/memories` con contenido en blanco | 400 `content must not be empty` |
| Búsqueda | `POST /v1/search` con la query "que puerto usa el daemon en desarrollo" (palabras distintas al texto guardado) | 1 hit, `distance 0.366`, `score 0.429`: **retrieval semántico real con el modelo ONNX** |
| Contexto | `POST /v1/context` | bloque `=== RECENT CONTEXT (Last 7 Days) ===` con la memoria |
| Apagado | `clawg stop --profile dev` | `daemon.json` y `token` eliminados, log `clawg serve dev stopped`, exit 0 |
| Registro | `claude mcp add clawg-dev -- <repo>\target\debug\clawg.exe mcp --profile dev` | agregado en scope local del proyecto (`~/.claude.json`); `claude mcp list` → `✔ Connected` |
| Autostart | el handshake de `claude mcp list` corrió `clawg mcp` sin daemon | el puente lanzó `serve` desacoplado (pid nuevo), que **siguió vivo después de que el puente cerró**; logs en `~/.clawg-dev/logs/serve.log` |
| **Sesión 1** | `claude -p` (proceso propio, Haiku) con `save_memory`: "El bar de la esquina de Marco se llama Los Girasoles Violetas y cierra los martes", `semantic_fact`, importancia 9 | la tool devolvió `queued` |
| **Sesión 2** | `claude -p` (otro proceso, sin ningún dato de la sesión 1) con `search_memory "bar de Marco"` | respondió: "El bar se llama **Los Girasoles Violetas**, cierra los **martes**, y el ID de la memoria es **#2**" |
| Fila resultante | `GET /v1/memories` | `id 2`, `agent_id: "claude-code"` (vino del `clientInfo.name` del initialize), `project_id: "claw-garden-0c70"` (`derive_project_id(cwd)`), `importance: 9.0` |

**Criterio de salida de la fase 1: cumplido.** Claude Code recordó algo entre dos sesiones distintas, sin la app de
escritorio abierta, contra el perfil dev, con autostart del daemon incluido.

Al cierre de la verificación el daemon se apagó con `clawg stop --profile dev`: hasta la fase 4, desktop y daemon
no deben correr a la vez sobre el mismo perfil. El puente lo vuelve a levantar solo la próxima vez que Claude Code lo necesite.

**Observaciones de la ejecución real** (no aparecieron en tests):

1. **Los archivos `daemon.json` y `token` quedan `rw-r--r--` en Windows.** Es lo que la bitácora ya decía (el HOME
   protege), pero visto en `ls -la` llama la atención. Si la fase 2 mueve datos fuera del perfil de usuario, revisar.
2. **`access_count: 1` en la fila recién guardada** antes de cualquier búsqueda. Viene del `GET /v1/memories`
   previo o del insert; no es un problema, pero el panel de la fase 4 no debería presentar ese 1 como "accedida una vez".
3. **La sección `=== RECENT CONTEXT ===` del prompt trae la memoria aunque la búsqueda densa también la devuelva.**
   Correcto según `build_memory_prompt` (buckets por recencia), pero `recall_context` y `search_memory` se pisan en
   los primeros 7 días. Es comportamiento heredado del chat, no del daemon.

### Regresión del 2026-09-29: el puente dejó de conectar con Claude Code 2.1.285

**Síntoma.** Al abrir una sesión nueva en este repo, `clawg-dev` falló con `CONNECT_TIMEOUT` a los 28 s. El daemon sí
arrancó (autostart del puente, `listening` en el mismo segundo). Nada del repo había cambiado desde el 13-09: cambió
el host. El log de Claude Code (`%LOCALAPPDATA%/claude-cli-nodejs/Cache/<proyecto>/mcp-logs-clawg-dev/*.jsonl`) lo dijo
sin ambigüedad, y resultaron ser **dos bugs independientes** que se tapaban entre sí.

| # | Bug | Causa raíz | Evidencia |
|---|-----|-----------|-----------|
| 1 | El puente moría en el handshake | Claude Code 2.1.285 abre cada sesión stdio con un request `server/discover` (id `server-discover-probe-1`) y cae a `initialize` sólo si el probe vuelve con error. rmcp 1.3 acepta como primer request únicamente `initialize` o `ping`; cualquier otro termina el servidor con `ExpectedInitializeRequest` (`service/server.rs:189`). El puente salía sin responder | stderr en el log: `expect initialized request, but received: ... server/discover`. engram (Go) y notebooklm (Python) reciben el mismo probe, responden error y conectan en ~200 ms con `protocolEra: "legacy"` |
| 2 | El host no se enteraba de que el puente había muerto | En Windows `std::process::Command` llama a `CreateProcess` con herencia de handles activada, que copia **todo** handle heredable al hijo, no sólo su stdio configurado. El daemon autoarrancado heredaba el extremo de escritura del pipe de stdout del host (el puente lo tenía heredable porque así lo recibió de node) | `echo <initialize> \| clawg mcp \| cat` tardó **67.075 ms** y terminó exactamente al hacer `clawg stop`. Por eso el bug 1 se vio como un timeout de 28 s y no como un fallo inmediato |

**Alternativas evaluadas para el bug 1.**

| Opción | A favor | En contra | Decisión |
|--------|---------|-----------|----------|
| `cargo update` dentro de 1.x (1.8.0) o subir a 2.2.0 | Barato | Verificado en el fuente: 1.8.0 y 2.2.0 tienen el mismo bucle de init que 1.3; no conocen `server/discover` | Descartada: no arregla nada |
| rmcp 3.5.0 sólo en `clawg-cli` (desktop sigue en 1.3; Cargo admite dos majors) | Soporte nativo del lifecycle nuevo (`DiscoverRequest`, `ServerHandler::discover`) | Dos majors de API en `server.rs` (830 líneas con macros). Y lo importante: en el lifecycle nuevo una sesión puede empezar **sin `initialize`**, y hoy `agent_id` sale de `clientInfo.name` del `initialize`. Migrar obliga a rediseñar la identidad del cliente | Diferida: decisión propia, no un hotfix |
| **Shim `PreInitGuard` sobre rmcp 1.3** | ~60 líneas, un solo build de rmcp, mismo camino de fallback que ya usan engram y notebooklm, no toca la derivación de identidad | Deuda explícita: se borra el día que el puente pase a rmcp 3.x | **Elegida** |

**Implementado** (dos commits, cada uno compila y pasa la suite solo):

- `fa048ad fix(cli): answer pre-initialize probes so MCP hosts can fall back to initialize` —
  `crates/clawg-cli/src/mcp/handshake.rs` (nuevo). `before_initialize(&msg) -> PreInit` es puro: `initialize` pasa y
  abre la sesión; `ping` pasa (rmcp ya lo contesta); un método que rmcp parsea como `CustomRequest` (o sea, que no
  implementa) recibe `-32601 method not found: <método>` con el id del request; un método conocido enviado antes de
  tiempo recibe `-32600 <método> sent before initialize`; notificaciones y respuestas sueltas se descartan.
  `PreInitGuard<T: Transport<RoleServer>>` aplica eso en `receive()` hasta ver `initialize` y después es transparente
  (post-init, contestar métodos desconocidos es trabajo del dispatcher de rmcp). `mcp/mod.rs` sirve sobre
  `PreInitGuard::new(AsyncRwTransport::new_server(stdin, stdout))` en vez de `stdio()` pelado.
- `1dd91b5 fix(cli): keep the autostarted daemon from inheriting the bridge's stdio on Windows` —
  `crates/clawg-cli/src/detach.rs` (nuevo, sólo `cfg(windows)`): `clear_inherit(&[RawHandle])` limpia
  `HANDLE_FLAG_INHERIT` (best effort, salta handles nulos) y `disinherit_std_handles()` lo aplica a stdin/stdout/stderr.
  `ProcessSpawner::spawn_serve` lo llama justo antes del `spawn`. `Command` sigue dándole al daemon su propio stdio
  (null + log) con handles que duplica a propósito. `windows-sys 0.61` como dependencia `cfg(windows)`: ya estaba en el
  lock vía `mio`, el lockfile sólo suma la arista (1 línea).

**Verificaciones.**

| Momento | Comando / acción | Resultado |
|---------|------------------|-----------|
| Rojo bug 1 | 10 tests de `mcp::handshake` contra un stub pass-through | 8 failed; el end-to-end cae en `the bridge is still alive` (el stream se cierra ante el probe: el bug, reproducido en proceso) |
| Verde bug 1 | ídem con la implementación | 10 passed |
| Rojo bug 2 | `an_inheritable_handle_stops_being_inheritable` contra `clear_inherit` vacío | failed en `!is_inheritable(handle)` |
| Verde bug 2 | ídem | 2 passed |
| Suite | `cargo test --workspace --all-features` | clawg-cli **104** + api 18, core 878 + 8, desktop 150, golden-mine 37, rag-eval 202 → **1.397, 0 failed** (1.385 + 12 nuevos) |
| Warnings | `cargo check --workspace --all-features` / `cargo clippy -p clawg-cli --all-targets` | 55 = baseline / 0 del crate |
| Binario real, bug 2 | el mismo `echo <initialize> \| clawg mcp \| cat` con el daemon parado | **649 ms** (antes 67.075 ms); el daemon autoarrancado sigue vivo después |
| Binario real, bug 1 | `server/discover` + `initialize` por stdin | `{"id":"server-discover-probe-1","error":{"code":-32601,...}}` y después el `InitializeResult` |
| Host real, en frío | daemon parado + `claude mcp list` (Claude Code 2.1.285) | `clawg-dev ... √ Connected`; log: `Successfully connected in 562ms`, `protocolEra: "legacy"`, `STDIO connection closed after 0s (cleanly)`; el daemon sobrevivió al `Terminating MCP server process tree` del host |
| **Criterio de salida, otra vez** | `claude -p --model haiku` con `search_memory "bar de Marco"` | "Los Girasoles Violetas, cierra los martes, ID #2"; conexión 189 ms, tool en 20 ms |

**Cosas aprendidas.**

- **El host es una dependencia que no está en `Cargo.lock`.** Nada del repo cambió entre el 13 y el 29 de septiembre;
  cambió Claude Code. Un criterio de salida verificado contra un host externo es una foto, no una garantía: hay que
  poder re-verificarlo barato (`claude mcp list` + una llamada `claude -p`) cada vez que el host se actualiza.
- **Ante un método desconocido, contestar error; nunca morir.** Es lo que permite que un cliente negocie hacia atrás.
  rmcp 1.x lo hace bien después de `initialize` y mal antes; los SDKs de Go y Python lo hacen bien siempre.
- **"Detached" en Windows no incluye los handles.** `DETACHED_PROCESS` suelta la consola; la herencia de handles es
  otra cosa y `Command` la deja siempre activa. Un hijo de larga vida lanzado desde un proceso con stdio en pipes se
  queda con esos pipes si nadie limpia el flag.
- **Un timeout largo puede ser un EOF que no llegó.** El síntoma "tarda 28 s" sugería performance; el cold start medido
  del puente era de 655 ms. Medir antes de optimizar evitó perseguir el problema equivocado.
- **Los logs de MCP del host son la primera fuente, no la última.** Muestran el stderr del server, el tiempo de
  conexión y la era de protocolo negociada; con ellos la causa del bug 1 salió en una lectura.

**Consideraciones que deja.**

- **Fase 2.** El checklist de release suma una prueba contra el host real sobre el binario **instalado**:
  `claude mcp list` (conexión en frío, con autostart) y una llamada `claude -p` que use una tool. La suite sola no
  habría visto ninguno de los dos bugs.
- **Migración a rmcp 3.x.** Cuando el puente migre: borrar `PreInitGuard` (y sus 10 tests) y decidir de dónde sale
  `agent_id` en una sesión que empieza con `server/discover` y nunca manda `initialize`. Esa decisión condiciona qué
  `agent_id` tienen las memorias guardadas desde entonces, así que va en conversación aparte.
- **Unix.** El bug 2 no aplica (los fds 0–2 del hijo se reemplazan y std abre todo con close-on-exec); `detach.rs` es
  sólo `cfg(windows)` a propósito.

---

## Fase 2 — Release e instalador (compuerta)

**Inicio:** 2026-09-29 · **Branch:** `feat/migration-claw-garden` · **Base:** `3ebae36` (fase 1 + regresión de handshake)

Alcance según el documento: bundle de Tauri con el identifier definitivo, versionado real, `clawg --version`, el
binario `clawg` en el PATH después de instalar, y `bundle.targets` fijado a lo que de verdad se produce. Criterio de
salida: Claw Garden instalado como aplicación, arranca contra `~/.clawg`, convive con la versión dev sin pisarla, y
**cero datos migrados**.

### Estado de partida (verificado 2026-09-29, leyendo fuentes, no documentación de segunda mano)

| Tema | Realidad | Consecuencia |
|------|----------|--------------|
| Identidad | `tauri.conf.json`: `Claw Garden` / `com.clawgarden.app`; flavor dev por merge patch (fase 0) | Nada que hacer: es la identidad definitiva |
| Versión | `[workspace.package] 0.1.0`, `tauri.conf.json` `0.1.0`, **`package.json` `0.0.0`**. `clawg --version` ya existe (clap `version`, con test) | Unificar y **proteger con un test**. Tauri cae a la versión de `Cargo.toml` si se quita `version` (doc de `tauri-utils 2.8.3`), pero no se depende de que el CLI resuelva `version.workspace = true` |
| `bundle.targets` | `"all"` → en Windows produce MSI (WiX) y NSIS | Sólo NSIS: instala por usuario sin admin (`%LOCALAPPDATA%`) y soporta hooks |
| Sidecar | `tauri-build 2.5.6` (`lib.rs:527`) procesa `bundle.externalBin` en **cada** build del desktop, no sólo en `tauri build`: exige que el archivo exista y lo copia a `target/<perfil>/<nombre sin triple>`. `TAURI_CONFIG` (lo que exporta `tauri build --config`) sí se mergea (`lib.rs:474`) | `externalBin` en la config base rompería `cargo test --workspace` sin el archivo y **pisaría `target/debug/clawg.exe`**, el binario que Claude Code tiene registrado como `clawg-dev`. Va en un overlay que sólo lee `tauri build` |
| Procesos durante el install | El template NSIS de Tauri (embebido en `@tauri-apps/cli` 2.10) sólo chequea `${MAINBINARYNAME}.exe` | Un `clawg serve` de prod vivo bloquearía el reemplazo de `clawg.exe` en un update o desinstalación. Hook `PREINSTALL`/`PREUNINSTALL` que lo apague |
| `clawg stop` | Manda `POST /v1/shutdown` y sale sin esperar | El proceso sigue reteniendo `clawg.exe` y `daemon.lock` unos instantes: el hook de NSIS no sirve y `clawg stop && clawg serve` falla con "another instance". `stop` tiene que esperar a que el proceso termine |
| PATH | NSIS no trae manejo de PATH; el plugin `EnVar` no viene con Tauri, y la manipulación de strings de NSIS trunca a `NSIS_MAX_STRLEN` (1024): el clásico instalador que borra el PATH del usuario | La lógica va en Rust (`clawg path add/remove`), con núcleo puro testeable, y el hook sólo la invoca |
| ONNX Runtime | `fastembed` con `ort-download-binaries`: no hay ninguna `.dll` en `target/debug` (linkeo estático) | Nada extra que empaquetar |
| **Subsistema del exe de escritorio** | `src-tauri/build.rs` emite `/SUBSYSTEM:console,10.0` (MSVC) y `main.rs` pide `windows_subsystem = "windows"` en release. Experimento en el scratchpad (crate de juguete con las dos cosas, lectura del header PE): **`CONSOLE 10.0`**, gana el link-arg | **Bug latente**: el primer release abriría una consola negra detrás de la ventana. Nunca se vio porque nunca se hizo un build de release |
| Toolchain | `x86_64-pc-windows-msvc`, rustc 1.91.1, `tauri 2.10.3`, `tauri-build 2.5.6`, `@tauri-apps/cli ^2.10.1` | El triple del sidecar es `x86_64-pc-windows-msvc` |

### Decisiones de diseño de la fase 2

- **Tres capas de config, cada una con un lector distinto.** `tauri.conf.json` (identidad, lo lee todo),
  `tauri.windows.conf.json` (bundler de Windows: targets NSIS, `installMode: currentUser`, hooks; Tauri la mergea sola
  en Windows y `tauri-build` no valida nada de eso) y `tauri.release.conf.json` (sólo `tauri build --config`:
  `externalBin` y el `beforeBuildCommand` que compila el sidecar). Así `cargo test` y `tauri dev` nunca ven el sidecar.
- **El sidecar se compila en release antes del bundle.** `scripts/build-sidecar.ts` corre
  `cargo build --release --locked -p clawg-cli` y copia `target/release/clawg.exe` a
  `src-tauri/binaries/clawg-<triple>.exe` (ignorado por git). `tauri-build` después lo copia de vuelta a
  `target/release/clawg.exe`: mismos bytes, inofensivo.
- **PATH por Rust, no por NSIS.** `clawg path add|remove [--dir]` (default: el directorio del propio ejecutable).
  Núcleo puro sobre el string (`with_entry`/`without_entry`: comparación sin distinguir mayúsculas ni `\` final,
  preserva el resto byte por byte); I/O de registro en `HKCU\Environment\Path` **preservando el tipo**
  (`REG_EXPAND_SZ` sigue siendo `REG_EXPAND_SZ`, o se romperían las entradas `%VAR%`), y broadcast de
  `WM_SETTINGCHANGE` para que las terminales nuevas lo vean sin cerrar sesión.
- **`clawg stop` espera a que el proceso termine.** Usa el `pid` que devuelve `/health` (vivo y verificado, no el de
  `daemon.json`), y en Windows espera el handle del proceso (`OpenProcess` + `WaitForSingleObject`), que es lo que
  suelta el archivo `.exe`. Timeout explícito, error legible si vence.
- **El subsistema sigue a `debug_assertions`, igual que `main.rs`.** `build.rs` emite `console,10.0` sólo cuando
  `debug_assertions` está activo y `windows,10.0` en caso contrario, así el atributo de `main.rs` y el link-arg
  dicen lo mismo.
- **Una sola fuente de versión, con guardia.** `[workspace.package]` manda; `tauri.conf.json` y `package.json` la
  repiten y un test de `clawg-desktop` falla si alguno se desalinea.
- **El instalador no toca Claude Code.** Registrar `clawg` (prod) es la etapa 5 de la fase 3, no un efecto del
  install. Hasta entonces el daemon de prod sólo corre si alguien escribe `clawg serve`.
- **Fuera de alcance, declarado:** firma de código (SmartScreen va a advertir; aceptable para uso personal),
  instalador para macOS/Linux, auto-update.

### Plan de trabajo

1. **Paquete A — correcciones previas al primer release** (TDD donde hay algo testeable): subsistema en `build.rs`
   (verificado con el crate de juguete: rojo `CONSOLE` → verde `WINDOWS`), guardia de versión, `clawg stop` que espera.
2. **Paquete B — `clawg path add|remove`** (TDD): núcleo puro, I/O de registro contra una clave descartable en los
   tests (nunca contra el `Environment` real), subcomando.
3. **Paquete C — receta de release**: `tauri.windows.conf.json`, `tauri.release.conf.json`,
   `scripts/build-sidecar.ts` (+ test de la parte pura), `src-tauri/windows/hooks.nsh`, scripts de `package.json`, `.gitignore`.
4. **Paquete D — compuerta** (build de producción + instalación: **requiere autorización de Marco**): `bun run tauri:build`,
   subsistema del exe instalado, `clawg --version`, PATH en una terminal nueva, `clawg status` contra `~/.clawg`
   (puerto 27380), convivencia con dev (`clawg-dev` sigue conectado, `~/.clawg-dev` intacto), y la prueba contra el host
   real que dejó la regresión del 29-09 (binario instalado con `--profile dev`, para no escribir nada en prod).

### Implementado (paquetes A, B y C)

| Commit | Qué |
|--------|-----|
| `631d761 fix(desktop): link release builds with the windows subsystem` | `src-tauri/build.rs`: con `PROFILE=release` emite `/SUBSYSTEM:windows,10.0` + `/ENTRY:mainCRTStartup`; si no, `/SUBSYSTEM:console,10.0` como antes |
| `2d6c3bc build: align package.json with the workspace version and guard it with a test` | `package.json` `0.0.0` → `0.1.0`; `src-tauri/tests/release_config.rs` exige que `tauri.conf.json` y `package.json` declaren `CARGO_PKG_VERSION` |
| `4cef3d8 feat(cli): make clawg stop wait until the daemon process has exited` | `crates/clawg-cli/src/process.rs` (`Watched::open/wait`: `OpenProcess(SYNCHRONIZE)` + `WaitForSingleObject` en Windows, `kill(pid, 0)` en Unix) y `stop.rs` (`stop_in`: pid de `/health` → handle abierto **antes** del shutdown → espera; `STOP_TIMEOUT` 15 s; `ClientError::StopTimeout { pid, timeout }`) |
| `d7af972 feat(cli): add clawg path to manage the user PATH on Windows` | `crates/clawg-cli/src/user_path.rs` (`with_entry`/`without_entry` puros; `registry::{add, remove}` sobre `HKCU\<subkey>\Path` como valor crudo; `broadcast_environment_change`) y el subcomando `clawg path add` / `clawg path remove [--dir]`, idempotente. `winreg 0.55` (ya en el lock) |
| `63926b7 build: add the Windows release recipe with clawg bundled as a sidecar` | `tauri.windows.conf.json`, `tauri.release.conf.json`, `scripts/build-sidecar.ts` (+ `.test.ts`), `src-tauri/windows/hooks.nsh`, scripts `build:sidecar` / `build:release` / `test:scripts` / `tauri:build`, `/src-tauri/binaries` en `.gitignore`, 4 guardias más en `release_config.rs` |

### Verificaciones realizadas (paquetes A, B y C)

| Momento | Comando / acción | Resultado |
|---------|------------------|-----------|
| Subsistema, rojo | crate de juguete con el `main.rs` y el `build.rs` reales, lectura del header PE | release → `CONSOLE 10.0` |
| Subsistema, primer intento | elegir por `CARGO_CFG_DEBUG_ASSERTIONS` | release `WINDOWS`, pero **dev no linkea** (`LNK2019 WinMain`): Cargo no expone esa variable a build scripts. Descartado (caso inesperado 1) |
| Subsistema, verde | mismo crate, `target/` limpio | bin dev `CONSOLE 10.0` · bin release `WINDOWS 10.0` · `cargo test` y `cargo test --release` linkean y pasan |
| Versión | `release_config` | rojo `left: "0.0.0" right: "0.1.0"` → verde |
| `process` | 3 tests contra procesos reales (`ping -n 30`) | rojo 2/3 con stub → verde 3/3 |
| `stop` | 3 tests (sin daemon, daemon publicado pero muerto, daemon que no termina) | rojo 3/3 → verde 3/3 |
| `stop`, binario real | daemon dev por autostart, `clawg stop`, `tasklist` inmediato | **0 procesos** al volver `stop`, `daemon.json` y `token` borrados. 356 ms con el embedder `ready`; 6.712 ms si se para durante la carga de ONNX (caso inesperado 2) |
| `user_path` | 9 tests puros + 3 de registro contra `HKCU\Software\clawg-test\*` | rojo 8/12 → verde 12/12; `REG_EXPAND_SZ` preservado en ida y vuelta; sin residuo en el registro |
| CLI | 3 tests de parseo de `clawg path` | rojo por compilación (E0599) → verde |
| Receta | 4 guardias nuevas en `release_config` | rojo 4/4 (archivos inexistentes) → verde; 6 en total |
| Script | `bun run test:scripts` | rojo (módulo inexistente) → 5/5 |
| Hooks NSIS | `makensis -V2 -WX` (el de `%LOCALAPPDATA%\tauri\NSIS`, el que usa Tauri) sobre un `.nsi` mínimo que incluye `hooks.nsh` con `LogicLib` | compila sin warnings |
| Config | `bunx tauri info` | parsea base + overlay de Windows sin errores |
| **Suite** | `cargo test --workspace --all-features` | clawg-cli **125** + api 18, core 878 + 8, desktop 150 + `release_config` **6**, golden-mine 37, rag-eval 202 → **1.424, 0 failed**; más 5 de `bun` |
| Warnings | `cargo check --workspace --all-features` / clippy de los crates tocados | 55 = baseline / 0 nuevos |

**Lo que NO se ejecutó, a propósito:** `clawg path add` contra el `Environment` real (habría puesto `target\debug` en el
PATH del usuario) y cualquier build de release del proyecto (`tauri build`, `cargo build --release`): eso es la compuerta.

### Casos inesperados (fase 2)

1. **Cargo no le pasa `debug_assertions` a los build scripts.** El primer arreglo del subsistema usaba
   `CARGO_CFG_DEBUG_ASSERTIONS`, que existe para otras cfgs pero se filtra para ésta. El verde de release escondía que
   en dev emitía `/SUBSYSTEM:windows` sobre un `main` de consola: `cargo test` del desktop habría dejado de linkear.
   Se vio sólo porque se probaron los dos perfiles. `PROFILE` sí es confiable y coincide con `debug_assertions` en los
   perfiles de stock; el workspace no define perfiles propios. Si algún día se define uno que herede de `release` con
   `debug-assertions = true`, `build.rs` y `main.rs` dejarían de coincidir.
2. **Parar el daemon durante la carga del embedder tarda lo que falte de esa carga.** `stop` ahora dice la verdad
   sobre cuánto tarda el proceso en terminar: el runtime de tokio espera la tarea bloqueante de ONNX antes de salir
   (6,7 s medidos contra 356 ms en régimen). `STOP_TIMEOUT` de 15 s lo cubre; si el instalador corre justo después de
   un autostart, el paso de `stop` puede demorar unos segundos. No es nuevo: antes pasaba igual y `stop` devolvía antes.
3. **`tauri-build` y `clawg-cli` escriben el mismo archivo.** `externalBin` en la config base habría hecho que cada
   build del desktop copiara el sidecar sobre `target/debug/clawg.exe`, el mismo archivo que produce `clawg-cli` y que
   Claude Code ejecuta como `clawg-dev`. Resuelto con el overlay y protegido por test.
4. **El template NSIS de Tauri no mira los sidecars.** `CheckIfAppIsRunning` sólo recibe `${MAINBINARYNAME}.exe`
   (leído del binario del CLI de npm, donde está embebido). De ahí los hooks de `stop`.
5. **Los tests de registro dejaban la clave padre vacía.** `HKCU\Software\clawg-test` quedaba después de borrar las
   hojas; el `Drop` ahora intenta borrarla también (falla, a propósito, mientras otro test paralelo la esté usando).

### Pendiente para cerrar la fase: la compuerta (paquete D)

Requiere un build de producción y una instalación real, así que **no se ejecuta sin autorización de Marco**. Checklist:

1. `bun run tauri:build` → compila `clawg` en release, el frontend y el desktop, y deja el instalador en
   `target/release/bundle/nsis/`.
2. Header PE de `target/release/clawg-desktop.exe` → `WINDOWS`; `target/release/clawg.exe --version` → `clawg 0.1.0`.
3. (Opcional, recomendado) copiar `~/.clawg-dev/models/` a `~/.clawg/models/` para evitar la descarga de 470 MB. Es el
   modelo, no datos del usuario: "cero datos migrados" se sostiene.
4. Instalar. Esperado: sin pedido de admin, en `%LOCALAPPDATA%\Claw Garden\`, advertencia de SmartScreen (sin firma).
5. Terminal **nueva**: `clawg --version`, `where clawg` → carpeta de instalación, `clawg status` → `profile: prod`,
   `data_dir: ~/.clawg`, `running: false`.
6. Abrir la app: sin consola detrás; crea `~/.clawg` con una `memory.db` vacía.
7. Convivencia: `claude mcp list` sigue mostrando `clawg-dev ✔ Connected`; `~/.clawg-dev` intacto; `bun run tauri:dev`
   sigue siendo `Claw Garden (Dev)` contra `~/.clawg-dev`.
8. Host real sobre el binario instalado, **en perfil dev** (para no escribir nada en prod): registrar temporalmente
   `"%LOCALAPPDATA%\Claw Garden\clawg.exe" mcp --profile dev`, `claude mcp list`, y desregistrarlo.
9. Update: correr el instalador otra vez con un `clawg serve` de prod vivo → el hook lo para, sin "error opening file".
10. Desinstalar → la entrada de PATH desaparece y `~/.clawg` queda. Reinstalar.

### Consideraciones para las fases siguientes (lo que la fase 2 deja a la vista)

- **Desktop y daemon sobre el mismo perfil siguen sin exclusión en código.** La regla "no corren a la vez hasta la
  fase 4" es una convención. En la fase 2 no molesta (nadie registra el MCP de prod), pero la etapa 5 de la fase 3
  registra `clawg` de prod mientras la app instalada puede estar abierta, y ahí serían dos procesos con su ingest
  worker sobre `~/.clawg/memory.db`. **Antes de la etapa 5** conviene que el desktop tome el mismo `daemon.lock` y se
  niegue a abrir la memoria si lo tiene el daemon (o al revés), con un mensaje que diga cuál cerrar. Es una decisión
  de comportamiento del desktop congelado: va en conversación aparte.
- **Los puentes `clawg mcp` también retienen `clawg.exe`.** Con el MCP de prod registrado (fase 3), un update con
  Claude Code abierto va a encontrar el archivo en uso por el puente, no por el daemon. El hook no puede pararlos sin
  matar sesiones ajenas; lo razonable es que el instalador avise. A resolver cuando exista el registro de prod.
- **Firma de código.** Fuera de alcance hoy; si Claw Garden sale de esta máquina, SmartScreen lo va a frenar.

### Cosas aprendidas (fase 2, hasta la compuerta)

- **Un verde en el perfil que te importa no prueba nada del otro.** El arreglo del subsistema pasó en release y
  rompía dev. Toda decisión en un build script que dependa del perfil se prueba en los dos.
- **Un crate de juguete es la forma barata de interrogar al linker.** Cinco líneas y leer dos campos del header PE
  contestaron lo que ninguna doc decía (qué `/SUBSYSTEM` gana) sin compilar el proyecto en release.
- **"Configuración del bundle" no significa "sólo la lee el bundler".** `tauri-build` actúa sobre `externalBin` en cada
  `cargo build`. Antes de poner algo en una config, averiguar quién la lee, no sólo quién debería.
- **Un comando que "pide" algo tiene que esperar a que pase, o nombrarse distinto.** `stop` que no espera no es un
  stop, es un `request-stop`; el costo aparece en el primer script o instalador que confía en el nombre.
- **La mitad del trabajo de un instalador es no romper lo que ya hay en la máquina.** PATH truncado, tipo de registro
  cambiado, un `.exe` en uso: ninguno es una feature, los tres son la diferencia entre instalar y dañar.

### Compuerta ejecutada (2026-09-30, autorizada por Marco)

Marco autorizó el build de producción, la instalación y el checklist completo, y eligió copiar el modelo desde dev.
Antes de tocar nada se exportó `HKCU\Environment` a un `.reg` de respaldo y se guardó el PATH de usuario como texto.

| Paso | Acción | Resultado |
|------|--------|-----------|
| 1 | `bun run tauri:build`, primer intento | el sidecar compiló y se copió bien (`clawg` release en 1m 28s); **falló el `ng build` de producción** por budget de estilos (caso inesperado 6) → `92b2a10` |
| 1 | `bun run tauri:build`, segundo intento | `target/release/bundle/nsis/Claw Garden_0.1.0_x64-setup.exe`, 22 MB. Warning de Tauri sobre el identifier (caso inesperado 7) |
| 2 | header PE y versión | `clawg-desktop.exe` → **`WINDOWS 10.0`**; `clawg.exe` → `CONSOLE` (correcto para un CLI), `clawg 0.1.0`; el sidecar staged es byte a byte `target/release/clawg.exe` |
| 3 | modelo | `~/.clawg-dev/models` → `~/.clawg/models` (465 MB) |
| 4 | instalar (`/S`) | exit 0 en 2 s, sin admin, en `%LOCALAPPDATA%\Claw Garden\` (`clawg-desktop.exe`, `clawg.exe`, `uninstall.exe`) |
| 5 | PATH y terminal nueva (PATH reconstruido desde el registro) | `REG_EXPAND_SZ` preservado, las 36 entradas previas idénticas, **una sola entrada agregada**; `where clawg` → la instalación; `clawg --version` → `0.1.0`; `clawg status` → `profile: prod`, `data_dir: ~/.clawg`, `running: false` |
| 6 | abrir la app | ventana "Claw Garden" (no "(Dev)"), **0 procesos `conhost` hijos**; creó `~/.clawg/memory.db` (schema 8) y `sessions/` |
| 7 | convivencia, con la app de prod abierta | `claude mcp list` → `clawg-dev ✔ Connected`, y el daemon dev (binario debug) corriendo **a la vez** que la app de prod. `~/.clawg`: 0 filas en `memory`, `episodic_trajectory` y `procedural_skill`. `~/.clawg-dev`: sus 2 memorias intactas |
| 8 | host real sobre el binario instalado, perfil dev, aislado con `--strict-mcp-config` | autostart en frío de un daemon **de release** (`AppData\Local\Claw Garden\clawg.exe serve --profile dev`); conexión 150 ms `legacy`; `search_memory` 21 ms → "Los Girasoles Violetas, martes, #2" |
| 9 | update con **dos** daemons vivos desde el `clawg.exe` instalado (prod y dev; app cerrada) | primero se detectó que el hook no cubría dev (caso inesperado 8) → `b9db085`, rebuild. Con el instalador nuevo: exit 0 en 4 s; los logs de los dos daemons muestran apagado ordenado; archivos reemplazados |
| 10 | desinstalar (`/S`) y reinstalar | carpeta eliminada; PATH **byte a byte idéntico** a la foto previa (comparación case-sensitive, `REG_EXPAND_SZ`, 36 entradas); `~/.clawg` intacto. Reinstalación: exit 0, `clawg` de vuelta en el PATH |

**Criterio de salida de la fase 2: cumplido.** Claw Garden está instalado como aplicación, arranca contra `~/.clawg`,
convive con la versión dev sin pisarla (probado con los dos corriendo a la vez) y no se migró un solo dato: la base
de prod tiene cero filas. Estado final de la máquina: app instalada, `clawg` en el PATH, ningún daemon corriendo,
`clawg-dev` registrado y conectando.

### Casos inesperados (compuerta)

6. **El primer `ng build` de producción de la historia del repo falló por budget.** `sidebar.component.css` pesa
   9,70 kB contra un error a los 8 kB (el default de Angular). Nadie lo había visto porque el build de producción del
   frontend nunca se había corrido (la fase 0 lo dejó como "no verificado"). Alternativas: recortar el CSS (trabajo de
   frontend fuera de alcance, sobre un componente que la fase 4 rehace como navegación del panel), compilar con la
   config `development` (no sería un release), o subir el error. Se subió el error a 12 kB y el warning quedó en 4 kB,
   así los excesos siguen visibles (`92b2a10`).
7. **Tauri advierte que `com.clawgarden.app` termina en `.app`**, que en macOS es la extensión de los bundles de
   aplicación. El documento de producto fija ese identifier como definitivo y macOS está fuera de alcance, así que
   no se cambió. Lo que queda atado al identifier en Windows es poco: los datos del usuario viven en `~/.clawg`, no
   en carpetas derivadas del identifier; sí quedaría huérfano el perfil de WebView2
   (`%LOCALAPPDATA%\com.clawgarden.app`). **Decisión pendiente de Marco**: mantenerlo, o cambiarlo (por ejemplo a
   `com.clawgarden.desktop`) antes de cualquier build para macOS o de distribuir el instalador fuera de esta máquina.
8. **Un daemon dev lanzado desde el `clawg.exe` instalado también bloquea el update.** El binario de release puede
   servir `--profile dev` (así ensaya la fase 3), y el hook sólo paraba prod. Se detectó en la prueba del paso 8 y se
   confirmó con un control negativo; los hooks ahora paran los dos perfiles (`b9db085`). Parar un daemon dev que
   corre desde otro lado es inofensivo: el próximo `clawg mcp` lo vuelve a levantar.
9. **El `cp` de MSYS no sirve como control negativo de "archivo en uso".** Sobre un `.exe` en ejecución, `cp -f`
   "funcionó": Cygwin/MSYS renombra el original y crea uno nuevo (semántica POSIX de unlink), y el proceso sigue con
   la imagen renombrada. NSIS abre el existente para escritura, que es otra operación. El control válido fue
   `[System.IO.File]::Open(path, Open, Write, None)` contra un daemon recién lanzado desde ese archivo: bloqueado.
10. **El PATH de usuario de esta máquina mide 1.294 caracteres.** Más que el `NSIS_MAX_STRLEN` de 1.024: la receta
    clásica de PATH en NSIS lo habría truncado al escribirlo. La decisión de hacerlo en Rust dejó de ser teórica.
11. **`claude mcp list` sondea todos los servers a la vez.** Con `clawg-dev` (debug) y el smoke test (release)
    registrados, el debug ganó la carrera del autostart y la prueba no ejercitó el binario instalado. La prueba
    aislada se hizo con `claude -p --strict-mcp-config --mcp-config <archivo>`, sin tocar `~/.claude.json`.
12. **NSIS preserva la fecha del archivo de origen.** `clawg.exe` instalado conserva la fecha del build del sidecar,
    así que la fecha no sirve para probar que un update lo reemplazó; la prueba es que los daemons se apagaron por el
    hook (sus logs) y que el instalador terminó con exit 0.

### Cosas aprendidas (compuerta)

- **Un control negativo que "falla" puede estar midiendo otra cosa.** El `cp` que sobrescribió un `.exe` en uso no
  probaba que no hubiera lock: probaba que MSYS no escribe como NSIS. Antes de concluir, reproducir la operación
  exacta del sistema bajo prueba.
- **La primera vez que corre un camino, aparecen deudas que ningún test veía.** El budget de CSS, el subsistema del
  exe y la advertencia del identifier existían desde antes del pivot; los reveló el primer build de producción.
- **Las pruebas contra el host tienen que aislar qué binario se está probando.** Con dos registros del mismo daemon,
  el resultado depende de quién gana una carrera. `--strict-mcp-config` convierte esa carrera en una prueba.
- **Foto antes, comparación byte a byte después.** El respaldo de `HKCU\Environment` y la comparación exacta del PATH
  fueron lo que permitió afirmar "la desinstalación deja la máquina como estaba" en vez de "parece que anduvo".
