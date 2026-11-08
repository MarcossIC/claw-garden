# setup-local-hooks — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 | S | — | ver [`state.yaml`](state.yaml) |

## Problema

`clawg setup claude-code` instala los hooks sólo en el `settings.json` del usuario: `read_state` toma esa ruta de `locations` (`crates/clawg-cli/src/setup.rs:645-658`, `:669-685`), y la CLI no tiene ninguna opción para otra carpeta (`Command::Setup`, `crates/clawg-cli/src/cli.rs:289-299`). Lo que escribe a nivel usuario afecta a todos los proyectos, y el [ROADMAP](../../../ROADMAP.md) pide que el piloto corra primero en un repositorio real. Hoy eso obliga a copiar a mano en `<repo>/.claude/settings.local.json` los dos grupos que arma `install_hooks` (`setup.rs:476-514`), con la ruta absoluta del binario escrita en JSON, y a borrarlos a mano después. Un error de tipeo en esa ruta deja la sesión sin contexto y sin aviso, y nada vuelve a revisar que el bloque sea el que `setup` escribiría.

Claude Code lee hooks de `.claude/settings.local.json` de la carpeta del proyecto, y una definición idéntica en dos archivos corre una sola vez (code.claude.com/docs/en/hooks, «Hook handler fields», leído el 2026-11-08). Así, los mismos hooks del plan en ese archivo y en el `settings.json` del usuario no se duplican.

## Enfoque

Una carpeta como destino de los hooks, y nada más:

- `clawg setup claude-code --local` instala en la carpeta donde se corre el comando; `clawg setup claude-code --path DIR`, en `DIR`. Las dos opciones se excluyen.
- Se escribe sólo `<DIR>/.claude/settings.local.json`, con los mismos hooks del plan, las mismas reglas de conflicto y de migración y la misma escritura, copia y reporte que el `setup` de usuario (R3, R5 y R7 de [`specs/setup`](../../../specs/setup/spec.md)).
- `--remove` con cualquiera de las dos quita de ese archivo los hooks de clawg del perfil. Si no queda nada más, borra el archivo, y la carpeta `.claude` si queda vacía.

## Alcance

1. Las opciones en la CLI y su rechazo con `setup cursor`, que instala para el usuario y no tiene destino por carpeta.
2. `crates/clawg-cli/src/setup/local.rs`: el plan puro sobre el archivo leído, la escritura y el reporte, reusando lo de `setup.rs`.
3. La carpeta personal se rechaza. Ahí abre una terminal nueva en Windows, y su `.claude` es el directorio de configuración del usuario.
4. La copia previa de cada archivo se nombra por su carpeta (`<carpeta>.settings.local.json-<stamp>`), para que la poda de cinco copias no mezcle repositorios.
5. Si git no ignora el archivo, el reporte lo dice: lleva la ruta del binario, que suele nombrar al usuario. Claude Code lo agrega a las exclusiones globales de git sólo cuando lo crea él (code.claude.com/docs/en/settings, «Keep personal settings out of a repository»).
6. Docs: `docs/reference/commands.md`, `README.md` y el piloto del ROADMAP.

## Fuera de alcance

- El servidor MCP y el bloque del protocolo. Siguen siendo del usuario: los registra `clawg setup claude-code` (o `claude mcp add`), y el bloque vive en el `CLAUDE.md` del usuario. Los hooks por carpeta suponen el servidor ya registrado.
- Escribir `.gitignore` o las exclusiones de git: el reporte avisa, no cambia archivos del repositorio.
- Buscar la raíz del repositorio. Se escribe en la carpeta indicada. En Windows, Claude Code lee el archivo de la carpeta donde arranca la sesión. En Linux y macOS, desde 2.1.211, lo lee de la raíz del repositorio y también el que haya en la carpeta de arranque.
- `setup cursor` por carpeta.

## Cómo se mide

Tests que hoy fallan porque las opciones no existen: el plan y el reporte en `setup/local.rs`, el análisis de las opciones en `cli.rs` y una instalación seguida de `--remove` sobre el binario en `tests/binary.rs`, que deja la carpeta como estaba. No toca el ranking, el esquema ni los datos guardados.
