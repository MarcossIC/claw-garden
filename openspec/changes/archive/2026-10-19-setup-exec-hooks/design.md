# setup-exec-hooks — Diseño

## D1. Forma exec siempre, con `clawg` a secas en 1

**Elegido:** el hook del plan es `command` + `args` (enfoque A de la exploración), y `clawg` sin argumentos sale con 1 (enfoque D).

**Alternativas:**
- **B, elegir la forma por la versión de `claude`.** `setup` tendría que ejecutar `claude --version`; el plan dejaría de depender sólo de lo leído; sin `claude` en el PATH habría que adivinar, y se conservarían `shell_word` y el analizador para escribir.
- **C, forma de shell con `"shell": "powershell"` en Windows sin Git Bash.** Ata el hook a una máquina y no resuelve ni los perfiles de shell ni el arranque del shell dentro de los 5 s.
- **Mantener la forma de shell** (la de client-setup, `openspec/changes/archive/2026-10-19-client-setup/design.md:41`). Su razón era que un cliente sin `args` correría el binario solo y clap saldría con 2. Esa razón la cubre D.

`args` existe desde Claude Code 2.1.139 (CHANGELOG de Claude Code: «Added hook `args: string[]` field (exec form) that spawns the command directly without a shell»); `setup` se verificó con 2.1.303. Un cliente anterior corre `command` sin `args`: `clawg` a secas imprime la ayuda en stderr y sale con 1, un error que no bloquea.

**D, en el código:** `main.rs` ya trata un error de uso de `clawg hook` como 1 (`main.rs:43-52`) con `is_hook_invocation` (`cli.rs:295`). Se agrega `is_bare_invocation` (un solo argumento, el programa) a la misma rama. Lo demás de clap no cambia: un subcomando mal escrito sigue en 2.

## D2. Qué es la forma heredada idéntica

**Elegido:** sólo la cadena que generaba `hook_commands` para ese evento y perfil, con cualquier ruta de clawg. Se reconoce partiendo la cadena con `split_command` (`setup.rs:326`), exigiendo exactamente `<clawg> hook <evento> --agent claude-code --profile P`, y volviendo a generar la cadena con la regla de comillas de la versión anterior desde la primera palabra: si coincide con el `command`, es idéntica.

**Alternativa:** cualquier cadena que `is_clawg_hook` reconoce. Reescribiría también una editada a mano (otro `--agent`, un argumento de más), y `setup` nunca reemplaza lo que el usuario cambió.

- La regla de comillas queda como `legacy_command`, usada sólo para reconocer; ya no escribe nada. Sin el `\` a `/` de Windows: la versión anterior ya lo había aplicado, así que la primera palabra de una cadena de Windows trae `/`.
- **Cualquier ruta:** una cadena idéntica con otra ruta de clawg (un binario movido) también se migra, a la ruta actual. La forma heredada es la que falla en PowerShell; dejarla como conflicto pediría a cada usuario un `--remove` y otro `setup`.
- **En su lugar:** el hook del plan reemplaza a la cadena dentro de su grupo, así el grupo conserva su posición y su matcher. Si el evento ya tiene el hook del plan, la heredada idéntica se quita (si su grupo queda vacío, el grupo también), para no correr dos veces.
- **Un conflicto congela el evento:** con un hook de clawg distinto (en cualquier forma) no se migra ni se agrega nada en ese evento; se reporta como hoy.

## D3. Reconocer lo propio

`is_clawg_hook` pasa a recibir el hook (`&Value`): con `args`, lee `command` como programa y `args` como argumentos; sin `args`, parte `command` como hoy. `remove_hooks` usa la misma función, así `--remove` quita las dos formas. Un conflicto se muestra como `command` seguido de los `args` (la función que hoy describe un servidor, `describe_server`, `setup.rs:269`, pasa a describir cualquier `command` + `args`).

## D4. Escritura

- **Destino real.** Si la ruta es un enlace simbólico (`symlink_metadata`), se escribe en `canonicalize(ruta)`; el temporal va en el directorio de ese destino, así el `rename` no cruza sistemas de archivos y no reemplaza el enlace. Un enlace roto falla al resolverse y se reporta como archivo que no se pudo escribir (exit 1), sin tocar nada.
- **Borrar un archivo enlazado** (el `CLAUDE.md` que sólo tenía el bloque, R6 de `specs/setup`) lo deja vacío: quitar el enlace o su destino rompería el gestor de dotfiles que lo puso ahí.
- **Modo.** En Unix el temporal se crea con `OpenOptions::mode` igual al del archivo real, y se fija otra vez con `set_permissions` para que el `umask` no le quite bits; así nunca existe más abierto que el original. En Windows no se copia nada: el único atributo es sólo lectura, y con él el `rename` falla igual que hoy.
- **`sync_all`** del temporal antes del `rename`: sin él, un corte de luz puede dejar el nombre apuntando a un archivo vacío (`sharing::save`, `crates/clawg-core/src/memory/sharing.rs:185-188`, tampoco lo hace; queda fuera de este cambio).
- **Copias.** Se crean con modo 0600 en Unix (`OpenOptions::mode`). El directorio sigue creándose con `create_dir_all`; su modo es de local-isolation (50).

**Alternativa descartada:** escribir en el lugar, sin temporal (F de la exploración): conserva enlace y modo sin código, pero un corte deja el archivo a medias.

## D5. Poda de copias

Después de escribir una copia se listan las de ese nombre (`<nombre>-` y 17 dígitos) y se borran todas menos las 5 más recientes. El sello `yyyyMMddHHmmssfff` ordena como texto, así que la decisión es una función pura sobre nombres (`stale_copies`), probada sin disco. Las copias de otro nombre y cualquier otro archivo del directorio no se tocan. Un borrado que falla se reporta como falla (exit 1) y no detiene el resto.

## Probar a mano

Con un home temporal, `CLAUDE_CONFIG_DIR` dentro de él y el perfil dev:

```sh
clawg setup claude-code --profile dev
jq '.hooks.SessionStart' "$CLAUDE_CONFIG_DIR/settings.json"
```

El hook tiene `command` = la ruta del binario y `args` = `["hook","session-start","--agent","claude-code","--profile","dev"]`. Con Claude Code real, en Windows sin Git Bash y una ruta con espacio, el contexto llega a la sesión.
