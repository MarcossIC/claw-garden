# client-setup — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 4 | S–M | memory-protocol (22), agent-lifecycle-hooks (25), archivados | ver [`state.yaml`](state.yaml) |

## Problema

Detalle en la [exploración](exploration.md); lo que sigue está verificado otra vez contra `develop` (`4a213aa`).

1. **Conectar clawg a Claude Code son pasos a mano, y el que fija la identidad es opcional.** `Command` no tiene ningún subcomando que registre el servidor o toque la configuración de un cliente (`crates/clawg-cli/src/cli.rs:32-162`). El README da `claude mcp add` sin `--agent` como paso principal (`README.md:312`) y la variante con `--agent claude-code` en una sección posterior (`README.md:330-340`). Sin `--agent`, el agente es lo que el cliente diga de sí mismo, y un cambio de ese nombre deja lo guardado fuera del dominio del agente nuevo (la tabla de la exploración).
2. **Los hooks de 25 no los instala nadie.** `clawg hook` existe (`crates/clawg-cli/src/hook.rs`), pero sólo se configura copiando a mano el JSON de «Probar a mano» del [diseño archivado](../archive/2026-10-18-agent-lifecycle-hooks/design.md#probar-a-mano), que además depende del PATH.
3. **El protocolo completo no llega al agente.** De `crates/clawg-cli/src/mcp/protocol.md` sólo viaja la apertura, como instrucciones del servidor (`opening`, `crates/clawg-cli/src/mcp/server.rs:122`, usado en `:1231`). Las secciones siguientes (tipos, importancia, resumen de sesión, después de compactar) no las carga ningún archivo que Claude Code lea solo.

## Objetivo que sirve

4: varios agentes, con el usuario controlando qué ve cada uno. Las reglas de `sharing.json` nombran agentes por texto (`crates/clawg-core/src/memory/sharing.rs`); sin una identidad que no se mueva, apuntan a nombres que cambian.

## Decisiones

El porqué de cada una está en el [diseño](design.md). Las decisiones abiertas de la exploración se cerraron con su recomendación.

- **`clawg setup claude-code [--profile P] [--dry-run] [--remove]`**, un subcomando con una tabla de un solo cliente.
- **Registro por el CLI de Claude Code** (enfoque C): `claude mcp add --scope user <servidor> -- <ruta absoluta> mcp --profile P --agent claude-code`. El estado previo se lee de `.claude.json` sin escribirlo. Sin `claude` en el PATH se imprime el comando exacto, se instala lo demás y sale con 1. `.claude.json` nunca se edita.
- **El agente es siempre `claude-code`**, el nombre que Claude Code reporta (`openspec/changes/archive/2026-10-13-mcp-protocol-upgrade/verify.md:57`): fijarlo igual conserva filas y workspace ([`specs/visibility`](../../specs/visibility/spec.md), R6).
- **Filas guardadas con otro nombre: nada** (opción c). Ni renombre ni alias; queda documentado cómo verlo con `clawg memories stats`.
- **Hooks siempre, a nivel usuario** (`settings.json` del directorio de configuración de Claude Code): `SessionStart` (`startup|resume|clear|compact`) y `UserPromptSubmit`, con el mismo `--agent` y `--profile` que el puente.
- **El protocolo completo, entre marcadores, en el `CLAUDE.md` de usuario.** El hook no lo repite (D3 de 25), así que no se duplica salvo la apertura, que también llega como instrucciones del servidor.
- **Nunca se reemplaza lo que no coincide.** Una entrada del servidor, o un hook de clawg del mismo evento y perfil, distinto del plan es un conflicto: se reporta y no se toca. Lo ajeno no se toca nunca. El bloque entre marcadores es generado y sí se reescribe.
- **`--remove`** quita el servidor (si es de clawg), los hooks de clawg del perfil y el bloque (salvo que el servidor del otro perfil siga registrado). Los datos de clawg no se tocan.
- **Perfil:** el de siempre (`--profile`, `CLAWG_PROFILE`, tipo de build); un build debug no registra `prod`. El servidor se llama como el perfil (`clawg`, `clawg-dev`).
- **No se parte el cambio.** El registro, los hooks y el bloque son un solo comando chico.

## Alcance

1. `clawg-cli`: `Command::Setup`, un módulo `setup` con el plan puro sobre el estado leído y un borde de E/S fino (leer, escribir atómico con copia previa, correr `claude`).
2. `clawg-core`: `paths::home_dir` público, para que `setup` resuelva el home igual que el resto (con `CLAWG_TEST_HOME` en un build de tests).
3. Docs: README (el paso de conectar pasa a ser el comando), `.claude/CLAUDE.md`, `openspec/README.md` y una spec nueva `specs/setup`.

## Fuera de alcance

- Otros clientes (Cursor, Codex, OpenCode).
- Renombre o alias de agentes.
- El texto del protocolo y los hooks en sí (22 y 25).
- Escribir `.claude.json` a mano.
- Servidores con el mismo nombre en scope `local` o `project`, que ganan al de usuario: se mencionan en el README, no se detectan.
- Avisar de una ruta registrada que dejó de existir (doctor-checks, 31).

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), todos sobre directorios temporales, sin tocar `~/.claude`, `~/.claude.json` ni `~/.clawg*`: el plan sobre un estado vacío; la segunda corrida no escribe; un servidor o un hook distintos se reportan y no se reemplazan; los hooks ajenos se conservan; el texto fuera de los marcadores no cambia y un marcador suelto es un error sin escribir; `--remove` deja el estado como antes; `--dry-run` no escribe; una ruta relativa se rechaza; sin `claude` se imprime el comando; y el binario real con un `claude` falso.

`rag-eval`: no aplica, no hay cambio de ranking.

## Datos y vuelta atrás

No toca el store, el esquema, los identificadores ni el contrato de embedding. Escribe sólo fuera del directorio de datos (`settings.json` y `CLAUDE.md` de Claude Code, y `.claude.json` a través de `claude`), y cada archivo que cambia se copia antes a `<data>/backups/setup/`. Volver atrás es `clawg setup claude-code --remove`, o restaurar esas copias.
