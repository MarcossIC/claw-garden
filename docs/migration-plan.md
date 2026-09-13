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
| 0 | Identidad, split del workspace, aislamiento de perfiles | ✅ implementada 2026-09-11 (sin commitear) | `clawg-core` compila sin `tauri` ✅; `cargo run` escribe en `~/.clawg-dev` y no puede abrir `~/.clawg` ✅ (por tests, no por ejecución); suite verde en el número baseline ✅ 998 + 37 + 202 |
| 1 | Núcleo: `clawg serve` + `clawg mcp` | ⚪ pendiente | Claude Code recuerda algo entre dos sesiones sin la app abierta, contra perfil dev |
| 2 | Release e instalador (compuerta) | ⚪ pendiente | Claw Garden instalado, arranca contra `~/.clawg`, convive con la versión dev. Cero datos migrados hasta acá |
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

**Puntos de commit sugeridos** (nada está commiteado; el orden respeta que cada commit compile y pase tests):

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
