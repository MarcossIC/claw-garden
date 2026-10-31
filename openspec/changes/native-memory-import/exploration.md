# native-memory-import — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 4 | M (la parte A); B y C son S | — para el importador, que construye sobre [history-import](../archive/2026-10-24-history-import/proposal.md) (44) y [import-undo](../archive/2026-10-24-import-undo/proposal.md) (84), archivados; la recomendación de apagar la memoria nativa espera al brazo A4 de [agent-eval](../agent-eval/exploration.md) (96) | exploración cerrada; sigue en la [propuesta](proposal.md) |

**Tesis:** Claude Code trae su propia memoria automática y la deja encendida por defecto. Para quien usa clawg, las dos memorias escriben en paralelo los mismos hechos, cada una con sus reglas, y lo que aprendió la nativa no le llega a otros agentes ni a otra herramienta. Las investigaciones proponen «integrarla en lugar de competir»: absorber lo que ya escribió y, recién con datos, recomendar apagarla. Este cambio propone un importador más (`clawg import --from claude-code`), sobre el modelo de notas ajenas que ya usa el de engram, y deja la decisión de apagar la memoria nativa a la medición de 96.

**De dónde sale.** De dos documentos de investigación previos, que no están en el repositorio: el diseño de integración («Reemplazo de MEMORY.md»: apagar con `autoMemoryEnabled`, comprobar que se respete y, como plan B, un importador que absorba `MEMORY.md`) y el informe de estado del arte (la memoria automática «compite directamente» con clawg; P1.10, «importar la `MEMORY.md` … para no competir con ella, sino absorberla»). Se contrastan con `develop` en 0.68.0 (`4604b35`) y con https://code.claude.com/docs/en/memory (sección «Auto memory», leída el 2026-10-31).

## Premisas de la investigación, verificadas

| Premisa | Resultado |
|---|---|
| La memoria automática escribe `MEMORY.md` en `~/.claude/projects/<proyecto>/memory/` | **Cierta y más precisa.** Es un índice, `MEMORY.md`, «one line per memory», y **un archivo por memoria** con frontmatter YAML cuyo campo `type` es `user`, `feedback`, `project` o `reference`; el cliente escribe la hora de cada escritura en un campo `modified` (ISO 8601). Lo que carga al arrancar son las primeras 200 líneas o 25 KB de `MEMORY.md`, no los archivos de tema. El directorio puede moverse con `autoMemoryDirectory` (página de memoria, «Auto memory», «Storage location») |
| Es por repositorio y compartida entre worktrees | **Cierta:** «all worktrees and subdirectories within the same git repository share one auto memory directory», local a la máquina (misma página). Coincide con clawg, donde un worktree resuelve al repositorio principal (`root_from_marker`, `crates/clawg-core/src/memory/identity.rs:144-159`) |
| Se apaga con `autoMemoryEnabled: false` o `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` | **Cierta,** también por proyecto en su `.claude/settings.json` o desde `/memory` (misma página, «Enable or disable auto memory») |
| Hay un issue de 2026 en que seguía escribiendo con todo apagado | **Sin verificar aquí.** No se abrió el issue; si se recomienda apagarla, la verificación es un paso del plan (B) |
| clawg no hace nada con la memoria nativa | **Cierta.** No hay ninguna coincidencia de `autoMemory` en `crates/`; `setup` sólo escribe el servidor, dos hooks y el bloque del protocolo (`HOOK_EVENTS`, `crates/clawg-cli/src/setup.rs:48`), y `doctor` no la mira |
| Habría que importar las notas «como candidatas» | **No existe un estado candidata, y no hace falta.** Una fila importada lleva `source = 'import'` y sale marcada `imported` en `memory_status`, ACTIVE CONFLICTS y `synthesize` ([`specs/provenance`](../../specs/provenance/spec.md), tabla de sellos). Es la misma marca que reciben las notas de engram |

## Lo que ya existe y se reutiliza

- El modelo de notas ajenas (`ForeignExport`, `ForeignNote`, `crates/clawg-core/src/memory/portability/foreign.rs:12-70`): un adaptador sólo lee su formato a este modelo, sin abrir el store.
- `translate` (`crates/clawg-core/src/memory/portability/translate.rs:308`): enmascara, parte con el tokenizer del daemon, tipa, fecha, deduplica por proyecto de origen y escribe `created_by_ref` y `origin_project`, que hacen que un segundo import encuentre cada nota.
- `--project-map NOMBRE=RUTA|NOMBRE=project:REF`, el diario del import y `clawg import --undo N` ([`specs/portability`](../../specs/portability/spec.md)).

## Partes

### A. `clawg import --from claude-code DIR` (Incorporar; M)

- **Lectura en el CLI:** DIR es el directorio de memoria de un proyecto, o `~/.claude/projects/` para todos. La CLI arma un documento JSON con cada archivo de tema (nombre, frontmatter crudo, cuerpo, `mtime`) y el nombre del directorio de proyecto, y lo manda por `POST /v1/import` con `from: claude-code`. El daemon sigue leyendo un solo texto, como hoy.
- **Adaptador puro** (`portability/claude_code.rs`): cada archivo de tema es una nota; el título es su `name` o el nombre del archivo; la fecha, `modified`, si no la hora del archivo; el origen, `claude-code:<proyecto>/<archivo>`.
- **Tipos:** `user` y `feedback` → `user_preference`; `project` y `reference` → `semantic_fact`; sin `type`, `semantic_fact`. Ninguna sale `architectural_decision`: la nativa no distingue una decisión de un hecho, y una decisión mal tipada se pone en disputa con las del usuario (`protocol.md:97-99`).
- **Proyecto:** el nombre del directorio es la ruta del repositorio con los separadores reemplazados, que no siempre se puede invertir. Se usa como nombre de proyecto de origen y se ubica con `--project-map`, como engram; sin mapa, la nota queda sin proyecto y `clawg projects move --origin-project` la ubica después.
- **`MEMORY.md` no se importa como nota:** es un índice de líneas que apuntan a los archivos de tema. Una línea que no apunta a ningún archivo se cuenta en `unplaced`.
- **Cómo se mide:** fixtures con un directorio de memoria real recortado (con su frontmatter tal cual) y los tests que ya tiene el import de engram por analogía: un segundo import no agrega nada, `--dry-run` no escribe, `--undo` lo deja todo como estaba.

### B. Apagar la memoria nativa (Posponer hasta 96)

`setup` podría escribir `autoMemoryEnabled: false` en el `settings.json` del usuario, que ya edita con copia de respaldo. Pero si A3 (clawg) no le gana a A4 (la memoria nativa con los mismos hechos) en 96, apagarla le quita algo al usuario sin dar nada a cambio. Cuando 96 conteste: como mucho una opción explícita de `setup` (nunca por defecto) y la comprobación de que el cliente la respeta.

### C. Avisar en `doctor` (Incorporar; S)

Un hallazgo `info` cuando el directorio de memoria del cliente tiene archivos de tema del proyecto y clawg está registrado: nombra cuántos hay y el comando `clawg import --from claude-code … --dry-run`, sin leer su texto (las reglas de `doctor`: ids, conteos, rutas y nombres, nunca un texto). Es la forma de que el usuario sepa que tiene dos memorias.

## Descartado

- **Leer `MEMORY.md` en cada sesión** para inyectarlo con el contexto: el cliente ya lo carga, y duplicarlo sólo suma tokens.
- **Escribir en la memoria nativa** lo que guarda clawg (un espejo): son dos fuentes de verdad, con conflictos entre ellas que nadie resuelve.

## Decisiones abiertas

- **El agente de las filas importadas:** `migrated`, como engram (`MIGRATED_AGENT`, `translate.rs:29`), o `claude-code`, que las pondría en el RECENT y el workspace del cliente como si las hubiera guardado él. Recomendado `migrated`: el agente nombra al autor, y la nota no la escribió el puente.
- **Si `feedback` es siempre una preferencia.** La página la define como correcciones y enfoques que el usuario confirmó, que es lo que el protocolo guarda como `user_preference`; un `feedback` que es un hecho del proyecto quedaría mal tipado, y `update_memory` lo corrige.

## Riesgos

- **El formato no es un contrato.** La página describe los archivos, no los fija; puede cambiar entre versiones del cliente. El adaptador acepta lo que no conoce y lo cuenta, como el de engram, y los fixtures se regraban con cada versión que cambie.
- **Secretos.** Las notas pasan por el mismo enmascarado que cualquier save; lo que no tenga un formato conocido queda como está, igual que en engram.

## Esfuerzo e impacto

| Parte | Veredicto | Impacto | Riesgo | Esfuerzo |
|---|---|---|---|---|
| A | Incorporar | Medio: hoy lo de la nativa no lo ve ningún otro agente | Bajo (todo el camino del import ya está probado) | Medio |
| B | Posponer | Según 96 | Medio (le quita algo al usuario) | Bajo |
| C | Incorporar | Bajo a Medio | Bajo | Bajo |

## Fuentes

- Código y specs de `develop` en `4604b35` (0.68.0), con las rutas citadas.
- https://code.claude.com/docs/en/memory, leída el 2026-10-31: «CLAUDE.md vs auto memory», «Auto memory», «Enable or disable auto memory», «Storage location».
- Los dos documentos de investigación previos, para la motivación.
