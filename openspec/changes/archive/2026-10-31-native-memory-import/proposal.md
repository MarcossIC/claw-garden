# native-memory-import — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 4 | M | 44 (history-import) y 84 (import-undo), archivados; apagar la memoria nativa espera a 96 (agent-eval) | ver [`state.yaml`](state.yaml) |

## Problema

Verificado en la [exploración](exploration.md#premisas-de-la-investigación-verificadas). Claude Code trae una memoria automática encendida por defecto: un índice `MEMORY.md` y un archivo Markdown por memoria en `~/.claude/projects/<proyecto>/memory/`, cada uno con un frontmatter que nombra la memoria y su tipo. Para quien usa clawg, las dos memorias guardan en paralelo, y lo que aprendió la nativa no lo ve ningún otro agente. Hoy clawg no la lee: `clawg import` conoce dos fuentes, `clawg` y `engram` (`ImportSource`, `crates/clawg-cli/src/cli.rs:320-326`; `Source`, `crates/clawg-cli/src/daemon/api.rs:2055-2059`), y `doctor` no mira el directorio del cliente (`run`, `crates/clawg-cli/src/doctor/mod.rs:139-219`).

Una diferencia con lo que dice la exploración: la página del cliente pone el tipo en `type`, pero las instrucciones de memoria que el cliente usa hoy lo escriben anidado, en `metadata:` → `type:`. El adaptador tiene que leer los dos.

## Objetivo

1 (guardar y clasificar) y 4 (compartido): lo que la memoria nativa ya escribió entra en clawg, tipado, con su fecha y su proyecto, donde lo ven todos los agentes; y `doctor` avisa mientras haya memoria nativa sin importar.

## Decisiones

Cierran las [decisiones abiertas](exploration.md#decisiones-abiertas) con lo que recomienda la exploración; el detalle está en el [diseño](design.md).

1. **`clawg import --from claude-code DIR`.** DIR es el directorio `memory/` de un proyecto, el directorio del proyecto que lo contiene, o `~/.claude/projects/` para todos. Como cualquier import desde el job (`crates/clawg-cli/src/import.rs:364-369`), la CLI manda la ruta y el daemon lee: lista el directorio, lee cada `.md` y arma un documento que un adaptador puro traduce al modelo de notas ajenas (`ForeignExport`, `crates/clawg-core/src/memory/portability/foreign.rs:12-28`). De ahí en adelante el camino es el de engram: `translate`, el diario, `--project-map`, `--dry-run` y `--undo`.
2. **Cada archivo de tema es una nota.** El título es `name`, si no el nombre del archivo sin `.md`; el contenido, `description` y el cuerpo; la fecha, `modified`, si no la hora del archivo; el origen, `claude-code:<proyecto>/<archivo>`, estable entre importaciones, que es lo que hace que un segundo import no agregue nada. `MEMORY.md` no es una nota: es el índice, y una línea suya que no apunta a ningún archivo de tema se cuenta.
3. **El tipo se lee de `type` y de `metadata.type`.** `user` y `feedback` se guardan `user_preference`; `project`, `reference`, un tipo desconocido o ninguno, `semantic_fact`. Ninguna nota sale `architectural_decision`: la memoria nativa no distingue una decisión de un hecho, y una decisión mal tipada se pone en disputa con las del usuario.
4. **Lo que el adaptador no conoce se cuenta, no se rechaza.** Una clave de frontmatter desconocida, un frontmatter sin cierre, una entrada que no es un `.md` y una línea del índice sin archivo van a `unplaced` por nombre, como en engram (`count_extra`, `crates/clawg-core/src/memory/portability/engram.rs:581-588`). Un tipo desconocido se ve en «notas por tipo de la fuente».
5. **Agente `migrated`** (`MIGRATED_AGENT`, `crates/clawg-core/src/memory/portability/translate.rs:29`), como engram: el agente nombra al autor, y la nota no la guardó el puente de Claude Code.
6. **El proyecto de origen es el nombre del directorio** (la ruta del repositorio con cada carácter que no es letra ni dígito cambiado por `-`), que no siempre se puede invertir. Se ubica con `--project-map`; sin mapa, la nota queda sin proyecto y `clawg projects move --origin-project` la ubica después. Como ese nombre empieza con `-` para una ruta absoluta, `--project-map` acepta un valor que empieza con guion.
7. **`doctor` avisa con un hallazgo `info`** (`native_memory`, `not_imported`) cuando Claude Code tiene registrado el servidor del perfil y hay archivos de tema cuyo origen ningún import de clawg tiene. Nombra cuántos proyectos y archivos, y el paso `clawg import --from claude-code <dir> --dry-run`. Lee nombres de archivo, nunca su texto.

## Alcance

- `crates/clawg-core/src/memory/portability/claude_code.rs`: el documento, su lectura del disco y el adaptador puro (frontmatter, índice, notas).
- `translate`: los tipos de `claude-code`.
- El daemon (`from: "claude-code"` en `POST /v1/imports` con una ruta de directorio y en `POST /v1/import` con el documento) y la CLI (`--from claude-code`, el informe sin versión de formato).
- `doctor`: el hallazgo y los orígenes importados que lee del store (`inspect`).
- `docs/reference/commands.md`, `docs/reference/daemon.md`, `specs/portability` y `specs/doctor`.

## No-alcance

- **Apagar la memoria nativa** (parte B de la exploración): espera a que 96 compare clawg contra ella.
- `autoMemoryDirectory`: `doctor` mira el lugar por defecto (`$CLAUDE_CONFIG_DIR/projects` o `~/.claude/projects`); un directorio movido se importa nombrándolo.
- Ubicar los proyectos sin `--project-map` invirtiendo el nombre del directorio.
- Escribir en la memoria nativa o leer `MEMORY.md` en cada sesión (descartados en la exploración).

## Cómo se mide el éxito

Tests que hoy fallan, con fixtures armados a mano y el embedder falso, sin modelo real, sin red y sin tocar un perfil de datos:

- El adaptador: un frontmatter con `type`, otro con `metadata.type`, uno sin frontmatter, uno sin cierre, claves desconocidas, fechas de `modified` y de la hora del archivo, el índice con líneas que apuntan y que no, los tres tipos de DIR y uno que no es ninguno.
- `translate`: los cuatro tipos y uno desconocido.
- De punta a punta por el binario: `--dry-run` no escribe nada; el import guarda las notas con el agente `migrated` y los tipos pedidos; un segundo import no agrega nada; `--undo` deja el store como antes del primero; `--map-root` es un error de uso.
- `doctor`: avisa con archivos sin importar y servidor registrado, calla sin servidor o sin archivos, y pasa a `ok` cuando todo está importado.

## Datos y vuelta atrás

No cambia el esquema, ningún estado ni ningún identificador: las filas importadas son las de un import ajeno (`source = 'import'`, `created_by_ref` con el origen). Un import se retira con `clawg import --undo N` y, como todo import real, deja antes el snapshot `pre-import`. Para volver atrás el código, se revierten los commits; las filas ya importadas siguen siendo filas válidas de un import ajeno.

## Riesgos

| Riesgo | Mitigación |
|---|---|
| El formato no es un contrato y cambia entre versiones del cliente | El adaptador cuenta lo que no conoce en vez de rechazarlo; los fixtures se rehacen con el formato que cambie |
| Un secreto en una memoria nativa | Pasa por el mismo enmascarado que cualquier save, antes de partirse (`translate`) |
| Un `feedback` que es un hecho del proyecto queda como preferencia | El informe lo muestra por tipo; `update_memory` lo corrige |
