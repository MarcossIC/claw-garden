# install-smoke — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | — (conviene después de 36 y 39, que son los primeros en ganar una prueba de proceso; ver «Interacciones») | exploración (borrador) |

**Tesis:** todas las pruebas de integración levantan el daemon y el puente dentro del mismo proceso del test. Ninguna lanza el binario `clawg`. Lo que se rompe entre el código y la máquina del usuario no lo cubre ningún test:

- el lock entre procesos;
- los archivos `daemon.json` y `token`;
- el framing de stdio con un host real;
- el autoarranque;
- lo que sobrevive a matar el proceso;
- el reinicio del daemon con un puente abierto.

Hoy eso sólo se verifica a mano, en cada `verify.md`. El defecto del token viejo del puente (39) es justo de esa clase: vivió varias versiones porque ningún test lo podía ver. Además, en Linux sólo se prueba lo que se compila a mano, y no hay ninguna compuerta que corra sola.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| Las pruebas de integración no lanzan el binario | **Cierta.** `crates/clawg-cli/tests/` tiene sólo `api.rs`, que levanta `axum::serve` en proceso con `MemoryRuntime::new_for_test` y lo dice: «The daemon binary is never spawned» (`crates/clawg-cli/tests/api.rs:1-9`). No aparece `CARGO_BIN_EXE_clawg` en ningún crate. Los únicos `CARGO_BIN_EXE_*` son de `golden-mine` y `rag-eval` (`crates/clawg-eval/tests/synth_roundtrip.rs:27,53,87`; `docs_mode.rs:14`). |
| Ningún test mata el daemon y revisa los datos | **Cierta.** Los únicos `kill` de producción terminan un proceso auxiliar de la prueba de procesos (`crates/clawg-cli/src/process.rs:142,153`). La durabilidad ante un kill está razonada (`crates/clawg-core/src/memory/db.rs:186-203`), no probada. Además, `new_for_test` no usa WAL (`db.rs:1317`). |
| Los handshakes MCP se prueban sólo con streams simulados | **Cierta.** El legado (`initialize`, `2025-06-18`) está en `crates/clawg-cli/src/mcp/handshake.rs:167-169`, con tests en `:191`, `:366` y `crates/clawg-cli/src/mcp/mod.rs:291-327`. El que no tiene sesión (`2026-07-28` en `_meta`) está en `handshake.rs:148` y `crates/clawg-cli/src/mcp/server.rs:1301`, con tests en `handshake.rs:196`, `:407` y `:431`. Ninguno lanza `clawg mcp`. |
| No hay forma de apuntar el daemon a un directorio de datos aislado | **Cierta.** El perfil sólo elige entre `~/.clawg-dev` y `~/.clawg` (`crates/clawg-core/src/paths.rs:56`, `:100`, `:114-115`), bajo `dirs::home_dir()` (`paths.rs:159-165`). No existe `CLAWG_HOME` ni `CLAWG_DATA_DIR`. En Linux, `HOME` lo redirige. En Windows, `dirs` usa la API de carpetas conocidas: **sin verificar** si respeta `USERPROFILE`. |
| El daemon no puede usar un embedder falso | **Cierta.** `FakeEmbedder` existe sólo con `cfg(any(test, feature = "eval", feature = "testing"))` (`crates/clawg-core/src/memory/embedder.rs:407`). Los tests lo inyectan por el closure de carga de `bootstrap.rs` (`:370`, `:824`), pero `serve` no tiene flag, variable ni argumento que lo elija. |
| Hay una palanca para el modelo real sin red | **Cierta, parcial.** `CLAWG_HF_BASE_URL` apunta la descarga a un espejo (`crates/clawg-core/src/memory/model_download.rs:146-147`). El modelo se cachea en `<data>/models/multilingual-e5-small` (`model_download.rs:374`) y `model_is_cached` lo detecta (`:480`). Copiar el modelo a ese lugar evita la descarga. |
| No hay CI | **Cierta.** No existe `.github/` ni ninguna otra configuración de CI. quality-guards (32) ya lo había visto y dejó la pregunta abierta. |
| La instalación sólo existe para Windows | **Cierta.** `scripts/install.ps1` es el único instalador, y `README.md:131` dice que en Linux y macOS «esa ruta no está probada». |
| `--port 0` sirve para no chocar puertos | **Cierta.** Se elige un puerto libre y el real queda en `daemon.json` (`crates/clawg-cli/src/cli.rs:41`; `crates/clawg-cli/src/daemon/serve.rs:123`). |

## Problema (verificado contra el código)

Lo que no puede ver una prueba en proceso:

- **Matar el proceso.** Lo que promete ingest-outbox (36), que un `202` sobreviva a un kill, sólo se demuestra matando un proceso real. El test en proceso de 36 (abrir otro runtime sin apagado ordenado) prueba la lógica, no el sistema operativo.
- **Dos procesos.** El lock exclusivo (`memory.lock`, `memory.owner`), el puente que autoarranca un daemon, y un daemon que se reinicia debajo de un puente abierto (39).
- **El stdio real.** Una escritura accidental a stdout corrompe el protocolo (`mcp/mod.rs:3-9`). Un test con streams simulados no la ve si sale de un camino que el test no ejercita.
- **El binario que se entrega.** Flags, códigos de salida, textos de `status`, `stop`, `backup` y `restore` como los recibe el usuario.

## Lo que ya existe y se reutiliza

- `tempfile` y el patrón de directorios temporales (`api.rs:148`).
- `--port 0` y `daemon.json` para encontrar el puerto real.
- `FakeEmbedder` con la feature `testing`, y el closure de carga que ya aceptan los tests (`bootstrap.rs:370`).
- La caché del modelo y `CLAWG_HF_BASE_URL` para el camino real sin red.
- Los handshakes y sus textos esperados, que los tests en proceso ya fijan: el smoke repite las mismas aserciones a través del proceso.
- La regla del repo: «no matar procesos `clawg.exe` que estén corriendo». El smoke sólo termina el hijo que lanzó, por su handle y nunca por nombre.

## Enfoques posibles

### A. Tests con el binario real dentro de `cargo test` (recomendado como núcleo)

Un archivo `crates/clawg-cli/tests/binary.rs` que usa `env!("CARGO_BIN_EXE_clawg")`. Corre en cada máquina que corre la suite. Escenarios:

1. `serve --port 0` sobre un directorio aislado → save → `Child::kill` (TerminateProcess en Windows, SIGKILL en Unix) → `serve` otra vez → la búsqueda encuentra lo guardado. Hoy es RED para un save que respondió `202`.
2. Dos `serve` sobre el mismo directorio: el segundo se rechaza y nombra al dueño.
3. `stop` espera a que el proceso salga. `status` sale con 1 sin daemon.
4. `backup` → `restore` → las filas vuelven, y existe el snapshot `pre-restore`.
5. `clawg mcp` por stdio real: handshake legado y sin sesión, `tools/list` con las once herramientas, `memory_status` y una herramienta autenticada.
6. Reinicio del daemon con el puente abierto: la siguiente herramienta responde. Es RED hasta 39.

Lo que necesita, y es la decisión central:

- **Directorio de datos aislado.**
  - (a) Una variable de entorno nueva que reemplace la raíz. Es superficie nueva del binario, y tiene que respetar la regla de que un build debug nunca abre `~/.clawg`.
  - (b) Redirigir el home del proceso hijo. Funciona en Linux. En Windows está sin verificar.
- **Embedder.**
  - (a) Compilar el binario de prueba con `--features testing` y elegir el falso por variable: rápido, pero prueba un binario que no es el que se entrega.
  - (b) El modelo real copiado a la caché del directorio temporal, o servido por `CLAWG_HF_BASE_URL` desde una carpeta local: prueba el camino real, pero cuesta unos 470 MB en disco y una carga lenta.
  - (c) Las dos, según una variable: el falso por defecto y el real en la compuerta de CI.

  El falso no da vectores unitarios. Los umbrales calibrados no se comportan igual, pero el smoke mide supervivencia, no ranking.

### B. Scripts de smoke sobre el binario instalado (PowerShell y sh)

- **A favor:** prueba lo que la suite no puede: el instalador, el PATH y `clawg doctor` sobre una instalación de verdad.
- **En contra:** son dos implementaciones que mantener, y quedan fuera de `cargo test`. Sirve sólo para el instalador, y en Linux todavía no hay instalador.

### C. Un workflow de CI (Windows y Linux)

Corre la suite completa, y con ella A, en una matriz `windows-latest` / `ubuntu-latest`. Es lo único que corre sin que alguien se acuerde, y lo único que prueba Linux por sí solo.

- **Costo:** infraestructura nueva, y el modelo real en caché del runner si se elige (b).
- Es la decisión que quality-guards (32) dejó abierta. Acá se vuelve concreta.

**Recomendación:** A con un directorio aislado y el embedder (c). C como decisión del usuario. B sólo cuando exista un instalador para Linux.

## Descartado

| Opción | Razón técnica |
|---|---|
| Un subcomando `clawg selftest` en el binario | Es superficie de producto para algo que hace una prueba. quality-guards (32) ya lo descartó por eso. |
| Matar procesos por nombre en la prueba | Puede terminar el `clawg.exe` del editor del usuario, cosa que las reglas del repo prohíben. Sólo se termina el hijo propio, por su handle. |
| Probar el ranking a través del binario | Ya lo mide `rag-eval`, con el mismo worker del daemon (`EvalIngest`). El smoke mide que las cosas sobrevivan y respondan, no su calidad. |
| macOS en la matriz | No hay instalador ni nadie que lo use hoy. Se reabre si alguien compila ahí. |

## Interacciones

- **ingest-outbox (36).** El escenario 1 es su prueba de proceso. Con la cola en RAM falla para un `202`; con el outbox, pasa.
- **bridge-reconnect (39).** El escenario 6 es su prueba de proceso.
- **eval-rigor (33).** Una compuerta de recuperación en CI necesita un set legible en esa máquina. El set privado está ignorado (`.gitignore:31-32`), así que sólo serviría el set versionado y escrito a mano que propone 33 (D). Si se adopta C, `--fail-on-regression` sobre ese set es lo que correría en CI.
- **quality-guards (32).** `clippy -D warnings` y el test de `#[allow(dead_code)]` entran en el mismo workflow si se aprueba C.
- **client-setup (26).** Cuando exista, el smoke puede comprobar que `clawg setup` se puede repetir y deshacer sobre un home temporal.
- **doctor-checks (31).** `doctor` sobre un directorio aislado es un escenario más, sin abrir `memory.db`.

## Alcance candidato

1. `crates/clawg-cli/tests/binary.rs` con los escenarios 1 a 5. El 6 entra con 39.
2. El mecanismo de directorio aislado elegido, con su regla de perfiles intacta y su test.
3. El mecanismo de embedder elegido, sin que el binario de release cargue código de prueba.
4. Una guarda de limpieza: cada hijo lanzado se termina al terminar la prueba (también si la prueba falla), por su handle.
5. Si se aprueba C: un workflow con la suite en Windows y Linux, y la caché del modelo si hace falta.
6. `.claude/CLAUDE.md`, sección Testing: cómo correr el smoke y cuánto tarda.

### No-alcance

- Un instalador para Linux o macOS.
- Medir la calidad de la recuperación o la latencia a través del binario.
- Cambiar el comportamiento de `serve`, `stop` o `mcp`. Sólo se prueban, salvo el mecanismo de aislamiento.

## Decisiones abiertas

Son del usuario.

- [ ] **Aislamiento.** ¿Una variable de entorno nueva para la raíz de datos (superficie nueva del binario, con la regla de perfiles) o redirigir el home del hijo (sin verificar en Windows)?
- [ ] **Embedder.** ¿Falso con `--features testing`, modelo real en caché, o las dos según una variable (recomendado)?
- [ ] **CI.** ¿Se agrega un workflow (Windows y Linux)? ¿Con el modelo real cacheado?
- [ ] **Tiempo.** ¿El smoke corre siempre con `cargo test`, o sólo con una variable (por la carga del modelo real)?
- [ ] **Escenario 6.** ¿Entra con este cambio como `#[ignore]` hasta que cierre 39, o lo agrega 39?

## Cómo se mide

Tests RED primero:

1. Kill después de un `202 embedder_loading`: hoy el save se pierde. Pasa con 36.
2. El segundo `serve` sobre el mismo directorio se rechaza y nombra al dueño.
3. `backup`/`restore` de ida y vuelta, con el `pre-restore` presente.
4. Los dos handshakes por stdio real: las once herramientas y `memory_status`.
5. Reinicio del daemon con el puente abierto: hoy falla con el texto del token. Pasa con 39.
6. Si la prueba falla a la mitad, no queda ningún proceso hijo vivo.

`rag-eval` no aplica: el smoke no toca el ranking. Si se agrega el aislamiento por variable, un test fija que un build debug con esa variable apuntando a `~/.clawg` se rechaza, igual que hoy con `--profile prod`.

## Riesgos

- **Pruebas lentas o inestables.** Los procesos, los puertos y la carga del modelo suman tiempo y variación. Se mitiga con `--port 0`, esperas con tope y el embedder falso por defecto.
- **Superficie nueva del binario.** Una variable de directorio de datos la puede usar también un usuario. Tiene que documentarse y respetar la regla de perfiles.
- **Procesos huérfanos.** Un hijo que sobrevive a la prueba bloquea `target\debug\clawg.exe` en Windows y rompe el siguiente build. La guarda de limpieza es obligatoria.
- **Falsa seguridad.** El smoke prueba el binario compilado, no el instalado. El instalador sigue fuera hasta B.
