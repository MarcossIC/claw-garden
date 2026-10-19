# Claw Garden

Memoria local para agentes de código. Claw Garden guarda, clasifica y recupera memorias y contexto, y se los sirve a un agente (hoy Claude Code) mediante un daemon HTTP local y un puente MCP por stdio. Todo corre en tu máquina: SQLite para el almacenamiento y embeddings ONNX para la búsqueda semántica.

## Contenido

- [Instalar desde un release](#instalar-desde-un-release)
- [Actualizar](#actualizar)
- [Instalar en Windows](#instalar-en-windows)
- [Comprobar que funciona](#comprobar-que-funciona)
- [Ver lo que se guarda](#ver-lo-que-se-guarda)
- [Conectarlo a Claude Code](#conectarlo-a-claude-code)
- [Otros clientes MCP](#otros-clientes-mcp)
- [Si algo falla](#si-algo-falla)
- [Desinstalar](#desinstalar)
- [Referencia](#referencia)

## Instalar desde un release

Cada versión se publica en [GitHub Releases](https://github.com/MarcossIC/claw-garden/releases) con dos binarios sin comprimir (`clawg-x86_64-pc-windows-msvc.exe` y `clawg-x86_64-unknown-linux-gnu`), el script `install.ps1`, la lista `SHA256SUMS` y su firma `SHA256SUMS.minisig`.

El binario lleva adentro ONNX Runtime, el motor que calcula los embeddings, enlazado de forma estática. No es una build de Microsoft: es la que compila [pyke](https://ort.pyke.io/setup/linking), que `ort` baja de `cdn.pyke.io` al compilar y verifica contra el SHA-256 que `ort-sys` fija para cada plataforma.

### Windows

Hace falta el **Microsoft Visual C++ Redistributable (x64)**. El script comprueba que esté, y si falta te dice dónde bajarlo (<https://aka.ms/vs/17/release/vc_redist.x64.exe>) sin instalar nada. En una PowerShell:

```powershell
irm https://github.com/MarcossIC/claw-garden/releases/latest/download/install.ps1 | iex
```

El script baja el binario del último release, lo compara con `SHA256SUMS` y, si no coincide, se detiene sin tocar nada. Después hace lo mismo que se cuenta en el [paso 4](#paso-4-instalar) de más abajo: instala en `%LOCALAPPDATA%\Programs\clawg`, agrega esa carpeta al PATH del usuario y renombra el binario anterior a `clawg.exe.old-<fecha>`. Abrí una terminal nueva al terminar.

El hash de `install.ps1` prueba que lo que bajaste es el archivo publicado, no quién lo publicó. Para comprobar la autoría, verificá la firma como se explica en [Verificar lo que bajaste](#verificar-lo-que-bajaste).

**SmartScreen y Smart App Control.** El `.exe` no está firmado con un certificado Authenticode. Si lo bajás con el navegador, SmartScreen puede mostrar "Windows protegió su equipo" (*Más información*, *Ejecutar de todas formas*); el script lo baja con `Invoke-WebRequest`, que no marca el archivo como descargado de internet. Smart App Control (Windows 11) puede bloquear un ejecutable sin firma de confianza, y ese bloqueo no se salta desde el script: o lo desactivás, o [compilás desde el código fuente](#instalar-en-windows).

### Linux

Bajá el binario y su lista de hashes, verificalos y ponelo en tu PATH:

```bash
base=https://github.com/MarcossIC/claw-garden/releases/latest/download
curl -fLO "$base/clawg-x86_64-unknown-linux-gnu"
curl -fLO "$base/SHA256SUMS"
sha256sum --check --ignore-missing SHA256SUMS
install -m 755 clawg-x86_64-unknown-linux-gnu ~/.local/bin/clawg
```

El binario se construye en Ubuntu 24.04 y pide **glibc 2.39** o más nueva: Ubuntu 24.04, Debian 13 o Fedora 40 en adelante (`ldd --version` te dice la tuya). No enlaza OpenSSL. Sólo x86_64.

### Verificar lo que bajaste

Los hashes de `SHA256SUMS` están firmados con [minisign](https://jedisct1.github.io/minisign/). Las claves públicas viven en el repositorio, en `release/minisign.pub` (la de uso) y `release/minisign-next.pub` (la de respaldo). Con `minisign` instalado:

```bash
curl -fLO "$base/SHA256SUMS.minisig"
curl -fLO https://raw.githubusercontent.com/MarcossIC/claw-garden/main/release/minisign.pub
minisign -Vm SHA256SUMS -p minisign.pub
sha256sum --check --ignore-missing SHA256SUMS
```

La firma lleva un comentario con la versión (`clawg vX.Y.Z SHA256SUMS`). Cada release lleva además una atestación de procedencia de GitHub: `gh attestation verify <archivo> --repo MarcossIC/claw-garden`.

### Si una clave de firma se compromete

La clave privada de uso está cifrada con contraseña y sólo existe como secreto del entorno `release` de GitHub Actions, que se limita a la rama `main`. La clave de respaldo se guarda fuera de GitHub y está en el repositorio desde el primer release.

- **Si se pierde o se filtra la de uso**, se publica un release firmado con la de respaldo que ya no lista la clave comprometida.
- **Lo que la clave de respaldo no hace:** no protege a los binarios que no se actualicen. Siguen aceptando la clave filtrada hasta que instales un release cuya lista ya no la tenga, y quien tenga esa clave puede firmar un `SHA256SUMS` que ellos acepten. Mientras tanto, desconfiá de cualquier release que no hayas visto anunciado en el repositorio.
- Si se compromete la de respaldo, se publica un release firmado con la de uso que la saca de la lista.

## Actualizar

Un `clawg` instalado desde un release se actualiza solo, y sólo cuando se lo pedís: el daemon nunca consulta nada por su cuenta.

```
clawg update --check    # dice si hay una versión nueva; sale con 100 si la hay, 0 si estás al día y 1 si falla
clawg update            # la instala
```

`clawg update` baja el binario del último release junto al instalado, verifica la firma de `SHA256SUMS` con las claves del binario y el hash del archivo, y comprueba que el nuevo diga la versión esperada. Hasta ahí no toca nada: si algo falla, el binario, el daemon y tus memorias quedan como estaban. Después detiene el daemon (si había uno), aparta el binario actual como `clawg.exe.old-<fecha>` (`clawg.old-<fecha>` en Linux), pone el nuevo y arranca su daemon en el mismo puerto. Si no había daemon, no arranca ninguno.

- **Si el daemon nuevo no arranca**, vuelven el binario anterior y su daemon, y `clawg update` sale con 1 explicando por qué.
- **Si el daemon nuevo alcanzó a migrar el store y el anterior ya no lo abre**, `clawg update` restaura el snapshot `pre-migration` que escribió ese arranque y devuelve el binario anterior. El store migrado queda guardado como `pre-restore` en `backups`, así que nada se destruye. Si el restore también falla, queda instalado el binario nuevo (el único que abre ese store) y el informe dice dónde está cada archivo y qué comandos correr.
- **Si el store tiene migraciones pendientes**, el daemon nuevo escribe antes un snapshot `pre-migration` en `backups` y el informe final nombra su ruta. Volver a la versión anterior después de migrar significa restaurarlo, y se pierde lo que se guardó desde entonces.
- **Los editores abiertos** siguen con el puente anterior hasta que los reinicies; el puente abierto igual alcanza al daemon nuevo.

`clawg update` se niega, y dice por qué, en un build de desarrollo, en uno compilado a mano (no lleva la marca de un release) y en uno que corre dentro de un directorio `target` de cargo: esos se actualizan con git y cargo. En Linux no usa `sudo`: si no podés escribir donde está el binario, falla antes de bajar nada.

### Volver atrás a mano

Si querés volver a la versión anterior después de una actualización:

```
clawg stop --profile prod
```

Poné de nuevo el binario anterior en su nombre: renombrá `clawg.exe.old-<fecha>` (la carpeta de instalación es la de `clawg doctor`) a `clawg.exe`. Si la actualización migró el store, restaurá el snapshot de antes con ese binario:

```
clawg restore ~/.clawg/backups/memory-<fecha>-pre-migration.db --profile prod
```

`clawg restore` guarda el store actual como `pre-restore` antes de reemplazarlo. El siguiente comando que necesite el daemon lo arranca.

## Instalar en Windows

Esta sección compila desde el código fuente, por si preferís no usar el binario de un release o querés probar un cambio. Un script lo instala sólo para tu usuario, sin permisos de administrador. Son cinco pasos. Los comandos van en **PowerShell**.

### Paso 1. Instalar lo necesario

Necesitás tres cosas:

1. **Git**: <https://git-scm.com>.
2. **Rust**, con `rustup`: <https://rustup.rs>. Aceptá la opción por defecto, que instala el toolchain **MSVC**.
3. **Build Tools de Visual Studio**, con el componente **"Desarrollo para escritorio con C++"**. Si no lo tenés, `rustup` te lo pide al instalar.

Después comprobá que tenés el toolchain MSVC:

```powershell
rustup toolchain list
```

La lista tiene que incluir una línea `stable-x86_64-pc-windows-msvc`.

- **Si está en la lista**, seguí al paso 2.
- **Si no está**, instalalo y seguí:

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc
```

> **Por qué MSVC y no GNU:** el runtime de ONNX (`ort`) sólo trae binarios para el toolchain MSVC. Con el toolchain GNU la compilación falla con `ort does not provide prebuilt binaries for the target x86_64-pc-windows-gnu`.

### Paso 2. Descargar el código

```powershell
git clone <URL-del-repositorio> claw-garden
cd claw-garden
```

### Paso 3. Compilar

Primero comprobá qué `rustc` encuentra tu PATH:

```powershell
Get-Command rustc -All
```

- **Si la primera línea es `C:\Users\<tu-usuario>\.cargo\bin\rustc.exe`** (el de `rustup`), compilá:

  ```powershell
  rustup run stable-x86_64-pc-windows-msvc cargo build --release -p clawg-cli
  ```

- **Si la primera es otra**, por ejemplo `C:\msys64\mingw64\bin\rustc.exe`, hay otro Rust delante del de `rustup`. Seguí [Si tenés dos Rust instalados](#si-tenés-dos-rust-instalados).

Tarda unos minutos la primera vez y necesita internet. Al terminar existe el archivo `target\release\clawg.exe`.

`rustup run stable-x86_64-pc-windows-msvc` compila con el toolchain MSVC aunque tu toolchain por defecto sea otro, y sin cambiarlo. Lo que no hace es esquivar el PATH: el `cargo` que lanza busca `rustc` en el PATH y usa el primero que encuentra. Por eso la comprobación va antes.

#### Si tenés dos Rust instalados

Pasa cuando otro Rust, normalmente el de MSYS2, está antes que `rustup` en el PATH. Ese `rustc` es GNU, y la compilación falla con alguno de estos errores:

- ``could not find native static library `onnxruntime`, perhaps an -L flag is missing?``
- `E0514` / `compiled by an incompatible version of rustc`

`rustc -vV` lo confirma: dice `Built by MSYS2 project` o `host: x86_64-pc-windows-gnu`. Un `cargo clean` solo no lo arregla, porque la próxima compilación vuelve a usar el mismo `rustc`.

**Si no usás el Rust de MSYS2 para otra cosa**, desinstalalo desde la terminal de MSYS2:

```bash
pacman -R mingw-w64-x86_64-rust
```

Después abrí una PowerShell nueva y comprobá que `Get-Command rustc -All` muestre sólo el de `.cargo\bin`. Antes de compilar, borrá lo que haya quedado compilado con el otro `rustc`:

```powershell
rustup run stable-x86_64-pc-windows-msvc cargo clean --release
rustup run stable-x86_64-pc-windows-msvc cargo build --release -p clawg-cli
```

**Si lo necesitás**, decile a `cargo` qué `rustc` usar con la variable `RUSTC`, que tiene prioridad sobre el PATH. Todo en una línea:

```powershell
$env:RUSTC = rustup which rustc --toolchain stable-x86_64-pc-windows-msvc; rustup run stable-x86_64-pc-windows-msvc cargo clean --release; rustup run stable-x86_64-pc-windows-msvc cargo build --release -p clawg-cli
```

La variable vale sólo para esa ventana de PowerShell: si compilás desde otra, repetí la línea.

### Paso 4. Instalar

El script de instalación busca `clawg.exe` **al lado de sí mismo**, así que primero se copia ahí:

```powershell
Copy-Item target\release\clawg.exe scripts\clawg.exe; powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

Al terminar muestra la versión instalada (`clawg` y el número) e `Installed to ...`.

El script:

- copia `clawg.exe` a `%LOCALAPPDATA%\Programs\clawg`;
- agrega esa carpeta al PATH **de tu usuario** (no toca el del sistema);
- si ya había un `clawg` instalado, detiene su daemon y **renombra** el binario viejo a `clawg.exe.old-<fecha>` en vez de sobrescribirlo. Windows no deja sobrescribir un `.exe` que otro programa tiene abierto (un antivirus, el indexador o una sesión de Claude Code), pero sí renombrarlo. Esos `.old` se borran solos en la próxima instalación, o podés borrarlos a mano;
- se niega a continuar si hay otro `clawg` que taparía al nuevo, y al final comprueba que una terminal nueva resuelva `clawg` al archivo recién instalado.

El script no está firmado; por eso se lanza con `-ExecutionPolicy Bypass`, que vale sólo para ese comando. Es corto: podés leerlo antes en `scripts\install.ps1`.

Opciones del script, por si las necesitás:

- `-InstallDir <ruta>`: instalar en otra carpeta.
- `-SkipPath`: no tocar el PATH.
- `-RemoveLegacy`: desinstalar en silencio una instalación vieja de "Claw Garden" antes de instalar.

### Paso 5. Abrir una terminal nueva

El PATH actualizado sólo lo ven las terminales abiertas **después** de instalar. Cerrá la actual y abrí otra. Seguí con [Comprobar que funciona](#comprobar-que-funciona).

> **¿Linux o macOS?** El script y `clawg path` son sólo para Windows. En Linux x86_64 usá el binario del release ([Linux](#linux)); en otras plataformas podés compilar con `cargo build --release -p clawg-cli` y poner `target/release/clawg` en tu PATH a mano, pero esa ruta no está probada.

## Comprobar que funciona

En la terminal nueva, corré estos tres comandos uno por uno.

**1. La versión**

```powershell
clawg --version
```

Imprime `clawg` y la versión instalada. Si dice que no reconoce el comando, no abriste una terminal nueva o el PATH no se actualizó.

**2. El diagnóstico del PATH**

```powershell
clawg doctor
```

Muestra en JSON qué archivos `clawg.exe` encuentra tu PATH. Termina con código `0` si el primero es el que acabás de instalar y no hay copias duplicadas, y con código `1` si hay otra copia que lo tapa. No abre la base de datos, así que se puede correr cuando quieras.

**3. El estado del daemon**

```powershell
clawg status --profile prod
```

Recién instalado, lo normal es que diga `"running": false` y termine con código `1`: todavía nadie arrancó el daemon.

### Probar el daemon a mano

Abrí **dos** terminales. En la primera:

```powershell
clawg serve --profile prod
```

Dejala abierta. En la segunda:

```powershell
clawg status --profile prod
```

Ahora debería decir `"running": true`. La primera vez, el modelo de embeddings se descarga en segundo plano desde Hugging Face (una sola vez, necesita internet): mientras tanto el daemon ya responde, pero la búsqueda semántica todavía no está lista. Repetí `status` y vas a ver el modelo pasar de `loading` a `ready`; suele tardar entre 3 y 15 segundos.

Para detenerlo:

```powershell
clawg stop --profile prod
```

Si lo usás desde Claude Code no hace falta arrancarlo a mano: `clawg mcp` lo lanza solo cuando no hay uno corriendo.

## Ver lo que se guarda

`clawg memories` es un visor de sólo lectura para comprobar qué guardaron los agentes. **Necesita un daemon corriendo** (no lo arranca solo) y no modifica nada.

Los cuatro comandos:

```powershell
clawg memories stats --profile prod                # cuántas memorias hay, agrupadas por tipo, tier, estado, agente y proyecto
clawg memories list --profile prod                 # las más recientes primero (20 por defecto)
clawg memories search "base de datos" --profile prod   # busca; todas las palabras deben aparecer
clawg memories show 12 --profile prod              # una memoria completa, con todos sus metadatos
```

El número de `show` es el `id` que aparece en `list` y `search`.

`clawg memories list --deleted` lista las memorias borradas, la última borrada primero. Una borrada vuelve con `clawg undelete <id> --profile prod`, también con el daemon corriendo: se busca de nuevo como antes de borrarla. No vuelve si después se guardó el mismo texto (ya está esa copia), ni si otra memoria viva del mismo agente y proyecto tiene hoy su misma `topic_key` (ya hay una versión vigente de ese tema), ni si tiene un secreto (`clawg secrets` la lista y `clawg purge` la borra del todo).

Se pueden agregar estos filtros a `list` y `search`:

- `--here`: sólo las memorias del proyecto del directorio actual.
- `--project <ID>` y `--agent <ID>`: filtran por proyecto o por agente.
- `--type <T>`: filtra por tipo de memoria.
- `--tier <1|2|3>`: 1 es caliente, 2 tibia, 3 fría.
- `--status <active|disputed|superseded>`: estado del ciclo de vida.
- `--limit <N>` y `--offset <N>`: paginado; `--limit` va de 1 a 100.
- `--json`: salida en JSON, para scripts.

Detalles útiles:

- La búsqueda ignora mayúsculas y acentos: `configuracion` encuentra `configuración`.
- `show` mira por defecto la tabla `memory`; con `--kind episodic` o `--kind procedural` mira las otras. Los ids son por tabla.
- Un texto largo se guarda partido en fragmentos, y el visor los muestra como filas separadas.

## Conectarlo a Claude Code

Con Claude Code instalado (`claude` en el PATH), un solo comando conecta clawg:

```powershell
clawg setup claude-code --profile prod
```

Hace tres cosas, y se puede repetir sin miedo:

1. **Registra el servidor MCP** a nivel de usuario, así queda disponible en todos tus proyectos. Lo hace con el propio `claude mcp add --scope user`, con la ruta completa del `clawg` que corriste (para no depender del PATH del proceso que lanza Claude Code) y con la identidad del agente fijada en `--agent claude-code` (ver [Fijar la identidad del agente](#fijar-la-identidad-del-agente)).
2. **Instala los hooks** de inicio de sesión y de cada mensaje en `~/.claude/settings.json`: al empezar (y después de compactar) la sesión recibe la memoria del proyecto, y a los 30 minutos sin guardar el agente se entera. Los hooks que ya tenías no se tocan.
3. **Agrega el protocolo de memoria** (cuándo guardar, cómo escribir un save, qué no guardar) a `~/.claude/CLAUDE.md`, entre `<!-- BEGIN CLAWG MEMORY PROTOCOL -->` y `<!-- END CLAWG MEMORY PROTOCOL -->`. Lo que escribiste fuera de esas marcas no cambia; lo de adentro lo reescribe clawg cuando el protocolo cambia.

Antes de cambiar un archivo, guarda una copia en `~/.clawg/backups/setup/`. Con `--dry-run` muestra lo que haría sin tocar nada. Si `CLAUDE_CONFIG_DIR` está definido, usa ese directorio en vez de `~/.claude`.

Lo que no coincide con lo que instalaría no lo reemplaza: lo dice y sale con 1. Pasa, por ejemplo, si ya habías registrado `clawg` con otro comando o si moviste el binario. Para rehacerlo, `clawg setup claude-code --remove --profile prod` y otra vez `clawg setup claude-code --profile prod`. Si `claude` no está en el PATH, instala hooks y protocolo, imprime el comando de registro para que lo corras vos y sale con 1.

Un servidor llamado igual en el scope `local` o `project` de Claude Code gana al de usuario; `claude mcp list` muestra cuál se usa.

Después **abrí una sesión nueva** de Claude Code. Adentro:

1. Escribí `/mcp` para ver el estado del servidor `clawg`.
2. Pedile al agente que use la herramienta `memory_status`: te dice si el daemon está corriendo, qué perfil usa y si el modelo ya cargó.
3. Pedile que guarde algo, por ejemplo: "guardá en memoria que en este proyecto usamos pnpm".
4. En una terminal, comprobalo con el visor: `clawg memories list --here --profile prod`.

### Fijar la identidad del agente

Cada agente es dueño de sus memorias: las lista, las cambia, las borra y resuelve sus conflictos, pero no los de otro. Al buscar, también encuentra lo que guardaron los demás (ver [Compartir memorias entre agentes](#compartir-memorias-entre-agentes)). Sin configurar nada, el agente es el nombre que reporta el cliente MCP. Si el cliente cambia ese nombre, o deja de mandarlo (la versión nueva del protocolo lo vuelve opcional), lo guardado antes queda a nombre del agente viejo: la búsqueda lo sigue encontrando, marcado como de otro agente, pero el nuevo no lo puede listar, cambiar, borrar ni resolver como propio.

`clawg setup claude-code` lo fija siempre en `claude-code`, que es el nombre que Claude Code reporta de sí mismo, así que si venías sin `--agent` no perdés nada. Si registraste el servidor a mano con otro nombre, lo anterior queda con ese nombre: `memory_status` dice qué agente está en uso y de dónde salió, y `clawg memories stats` cuenta las memorias por agente. Nada se borra; vuelve a verse con el nombre anterior. Registrarlo a mano sigue siendo posible:

```powershell
claude mcp add --scope user clawg -- "$env:LOCALAPPDATA\Programs\clawg\clawg.exe" mcp --profile prod --agent claude-code
```

### Compartir memorias entre agentes

Todos los agentes leen lo que guardaron los demás: lo que Claude Code aprendió de vos le sirve a Cursor cuando busca. Lo compartido le llega al agente marcado `by claude-code`, como información para verificar y no como instrucciones, y es de sólo lectura: sólo su autor puede cambiarlo o borrarlo. El workspace y los conflictos de cada agente siguen siendo suyos.

Para que un agente no lea a otro, excluilo desde una terminal:

```powershell
clawg sharing hide cursor claude-code --profile prod
```

Desde la próxima búsqueda, `cursor` deja de encontrar lo que guardó `claude-code`, sin reiniciar nada. La exclusión va en una sola dirección: `claude-code` sigue leyendo a `cursor`. `*` vale por cualquier agente: `hide '*' codex` hace que nadie lea a `codex`, `hide codex '*'` que `codex` no lea a nadie, y `hide '*' '*'` vuelve a dejar a cada agente con lo suyo.

- `clawg sharing` muestra las exclusiones; `clawg sharing unhide cursor claude-code` quita una.
- `memory_status` dice a quién lee el agente.
- Los nombres son los que muestra `memory_status`, con las mismas mayúsculas.
- Una memoria compartida llega al proveedor del otro agente. `clawg secrets` lista las que guardan un secreto que el enmascarado reconoce.
- Si `sharing.json` queda inválido, ningún agente lee a otro hasta que lo arregles, y `memory_status` dice por qué.

Las herramientas que recibe el agente están en [Herramientas MCP](#herramientas-mcp).

## Otros clientes MCP

Cualquier cliente que acepte servidores MCP por stdio sirve: hay que darle el comando y sus argumentos. En los que se configuran con un JSON del estilo `mcpServers` (por ejemplo Cursor), sería:

```json
{
  "mcpServers": {
    "clawg": {
      "command": "C:\\Users\\<tu-usuario>\\AppData\\Local\\Programs\\clawg\\clawg.exe",
      "args": ["mcp", "--profile", "prod"]
    }
  }
}
```

El camino probado hoy es Claude Code. La configuración de otros clientes sigue el estándar MCP, pero no está verificada con cada uno.

## Si algo falla

**Al compilar: `ort does not provide prebuilt binaries for the target x86_64-pc-windows-gnu`.**
Estás compilando con el toolchain GNU. Usá el comando del paso 3 tal cual, con `rustup run stable-x86_64-pc-windows-msvc`. Si tampoco anda, tenés dos Rust instalados: seguí [Si tenés dos Rust instalados](#si-tenés-dos-rust-instalados).

**Al compilar: ``could not find native static library `onnxruntime` `` o errores `E0514` / `compiled by an incompatible version of rustc`.**
`cargo` usó el `rustc` de otro Rust (normalmente MSYS2) en vez del de `rustup`. Seguí [Si tenés dos Rust instalados](#si-tenés-dos-rust-instalados). Si ya lo sacaste del PATH y el error sigue, quedan restos compilados por él: corré los dos comandos de esa sección, `cargo clean --release` y la compilación.

**Al instalar el toolchain: `rustup toolchain install` falla con `detected conflict: 'bin\cargo.exe'`.**
Significa que el toolchain MSVC **ya estaba instalado** y rustup intentó actualizarlo. No hace falta reinstalarlo: seguí con el paso 2.

**Al compilar: error de `link.exe` o del linker.**
Faltan las Build Tools de Visual Studio con el componente de C++ (paso 1, punto 3).

**`clawg` no se reconoce como comando.**
La terminal es anterior a la instalación. Abrí una nueva.

**El instalador dice que otro `clawg` taparía al nuevo.**
Hay una instalación vieja. Repetí el paso 4 agregando `-RemoveLegacy`, o borrá a mano la copia que indica el mensaje.

**El instalador dice `Could not replace ... clawg.exe` o `user-mapped section open`.**
Tenés una versión vieja de `scripts\install.ps1`, que intentaba sobrescribir el binario. Actualizá el código (`git pull`) y repetí el paso 4. La versión actual renombra el binario viejo y no choca con este bloqueo.

**El instalador dice `Could not move the previous ... clawg.exe out of the way`.**
Algún programa tiene el binario viejo abierto sin permitir que se renombre. No se tocó nada. Fijate si es un `clawg` con `Get-Process clawg -ErrorAction SilentlyContinue | Select-Object Id, Path`. Si aparece, cerrá las sesiones de Claude Code que lo usan. Si no aparece ninguno, esperá un minuto. En los dos casos, después repetí el paso 4.

**`clawg memories ...` dice que no hay daemon.**
El visor no lo arranca solo. Corré `clawg serve --profile prod` en otra terminal, o abrí una sesión de Claude Code con el servidor registrado.

**Después de `clawg stop` o de actualizar, el agente dice que el daemon no está corriendo.**
El puente busca solo al daemon nuevo en cuanto vuelve a arrancar, pero no lo arranca él: así un `clawg stop` hecho a propósito (para `restore` o `purge`) se respeta. Corré `clawg serve --profile prod`, o en Claude Code escribí `/mcp` y reconectalo, que arranca uno con el binario instalado.

**Después de actualizar, `memory_status` dice `loading` un rato.**
Las versiones anteriores calculaban los vectores de las memorias de una forma que las dejaba casi todas igual de parecidas a cualquier pregunta. La primera vez que arranca esta versión guarda una copia en `~/.clawg/backups` (`…-pre-reembed.db`) y vuelve a calcular el vector de cada memoria; hasta que termina, la búsqueda por sentido no está disponible.

**`clawg doctor` termina con código 1.**
Hay otro `clawg.exe` en el PATH que gana, o una copia duplicada. El JSON que imprime dice cuál.

## Desinstalar

```powershell
clawg stop --profile prod
clawg path remove --dir "$env:LOCALAPPDATA\Programs\clawg"
clawg setup claude-code --remove --profile prod
Remove-Item -Recurse "$env:LOCALAPPDATA\Programs\clawg"
```

Esto, en orden: detiene el daemon, saca la carpeta del PATH, quita de Claude Code el servidor, los hooks y el protocolo que instaló `clawg setup` (si lo corriste) y borra el binario.

Tus memorias **no se borran**: viven en `~/.clawg`. Si también querés eliminarlas, borrá esa carpeta a mano.

## Referencia

### Compilar para desarrollo

```
cargo build -p clawg-cli
```

Produce el binario `clawg` (`target/debug/clawg`; `target/release/clawg` con `--release`). En Windows, usá el toolchain MSVC como en el paso 3.

### Comandos

```
clawg serve --profile dev     # el daemon de memoria
clawg mcp --profile dev       # puente MCP por stdio; arranca un daemon si no hay ninguno
clawg status --profile dev    # estado y salud del daemon en JSON (sale con 1 si no está corriendo)
clawg stop --profile dev      # detiene el daemon
clawg doctor                  # a qué clawg resuelve el PATH (sale con 1 si hay uno que tapa o está duplicado)
clawg update --check          # dice si hay un release nuevo (100 si lo hay); clawg update lo instala
clawg memories list|search|show|stats   # visor de sólo lectura (list --deleted: las borradas)
clawg undelete 12 --profile dev         # devuelve una memoria borrada (con el daemon corriendo)
clawg setup claude-code --profile dev   # conecta Claude Code: servidor MCP, hooks y protocolo (--dry-run, --remove)
```

`clawg mcp --no-autostart` falla en vez de arrancar un daemon, y `clawg mcp --agent <nombre>` fija el agente con el que guarda y busca. `clawg serve --port <n>` cambia el puerto por defecto (`0` elige uno libre). `clawg path add|remove [--dir <dir>]` edita el PATH del usuario en Windows.

### Registrar el servidor MCP en un build de desarrollo

Perfil `dev`, apuntando a un build debug (un build debug no registra `prod`):

```
/ruta/a/claw-garden/target/debug/clawg setup claude-code --profile dev
```

Registra `clawg-dev` con la ruta de ese binario y sus hooks con `--profile dev`; los de `prod` no se tocan, y el protocolo es el mismo bloque para los dos. Los nombres `clawg-dev` y `clawg` son las identidades que usa cada perfil.

### Herramientas MCP

- `save_memory`: guarda un hecho, preferencia o decisión para que sobreviva entre sesiones.
- `search_memory`: busca memorias relacionadas con una consulta, con ids, tipos y puntajes de relevancia. Lo que guardó otro agente va marcado `by <agente>`.
- `recall_context`: devuelve un bloque de contexto listo para leer (notas del workspace, memorias recientes y de largo plazo) para una consulta.
- `search_project`: busca en la documentación del proyecto (los Markdown y los manifiestos como `Cargo.toml` o `package.json`, respetando `.gitignore`) y en su código Rust, Python y TypeScript, partido por definición, y devuelve cada pasaje con su ruta de encabezados o de definiciones. Sirve para preguntas sobre cómo funciona el proyecto. El índice se arma solo la primera vez que el agente abre el proyecto y se pone al día cuando cambian los archivos.
- `list_memories`: recorre las memorias del agente, las más nuevas primero, sin buscar.
- `read_memory`: devuelve entero un texto largo que se guardó en varios pedazos, a partir del id de cualquiera de ellos; `search_memory` marca esos pedazos con `piece=K/N`.
- `delete_memory`: borra una memoria del agente por id y tipo.
- `restore_memory`: deshace un `delete_memory` del agente, con el mismo id y tipo.
- `update_memory`: corrige la importancia o el tipo de una memoria del agente. Si el campo que cambia era lo que la tenía en tier 1, la saca; lo que ganó por uso se queda.
- `resolve_conflict`: cierra un conflicto de ACTIVE CONFLICTS con la respuesta del usuario: una memoria queda y la otra pasa a reemplazada, o quedan las dos.
- `synthesize`: repasa las memorias en busca de conclusiones. Devuelve las que ninguna conclusión cubre todavía, las conclusiones ya sacadas y cómo guardar una nueva (con `save_memory` y `derived_from`). Las memorias en que se apoya una conclusión no cambian, y quedan marcadas como revisadas aunque no se guarde nada.
- `memory_status`: informa si el daemon está corriendo, su perfil y su directorio de datos, si el modelo de embeddings ya cargó, con qué agente guarda y busca, y a qué otros agentes lee.

### Perfiles y datos

- Perfil `dev`: datos en `~/.clawg-dev`, puerto por defecto 27381.
- Perfil `prod`: datos en `~/.clawg`, puerto por defecto 27380.

El perfil sale de `--profile dev|prod`, luego de la variable de entorno `CLAWG_PROFILE`, y por último del tipo de build (debug es `dev`, release es `prod`). Un build debug rechaza `prod`, así que nunca puede tocar `~/.clawg`. El daemon escucha sólo en 127.0.0.1; los endpoints `/v1` exigen un token bearer guardado en el directorio de datos (en Unix, con permisos 0600 desde que se crea). Toda ruta, `/health` incluida, responde 403 a una petición dirigida a un nombre que no sea `127.0.0.1`, `localhost` o `[::1]`, o que traiga una cabecera `Origin`: así una página web abierta en el navegador no llega al daemon, ni con DNS rebinding. No es una barrera contra un proceso del mismo usuario, que puede leer el token.

Cada directorio de datos contiene la base SQLite de memoria, `daemon.json` (pid, puerto, versión), `token`, y `memory.lock` / `memory.owner`, que garantizan que un único proceso sea dueño del almacén. Si se excluyó algo con `clawg sharing`, también `sharing.json`.

### Workspace

- `crates/clawg-core`: motor de memoria (almacenamiento, embeddings, ranking, workers en segundo plano).
- `crates/clawg-cli`: el binario `clawg`, con su daemon y el puente MCP.
- `crates/clawg-eval`: herramientas de evaluación, `rag-eval` y `golden-mine`.
- `docs/`: documentos de diseño y ejemplos.

### Tests

```
cargo test --workspace
```

Mirá [`.claude/CLAUDE.md`](.claude/CLAUDE.md) para las convenciones de contribución y de agentes.
