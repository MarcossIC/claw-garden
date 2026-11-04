# history-reimport — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 2 | M, operativo, sin código del producto | — | exploración; el ensayo es el paso siguiente |

**Tesis:** el historial entró a prod una sola vez, el 2026-10-24, con el release 0.62.0. Repetir esa importación sirve para dos cosas que hoy nadie sabe: si el trabajo de rendimiento funcionó en una importación real de punta a punta con un binario release, y dónde va lo que quedó sin proyecto. Se ensaya primero en el perfil `dev`, sobre una copia del store de prod, y recién después de aprobar el piloto se hace en prod.

## Problema (verificado)

**Lo que hay que comparar.** La línea base de prod es 54.436 filas en 1.146,9 s, con `embed` en el 97,2 %. Las mediciones posteriores se hicieron en `dev` y por partes. Ninguna repitió la operación entera en release.

**Lo que quedó sin proyecto.** 15.709 filas, el 28 % del store, salieron de 1.594 notas. Una fila sin proyecto la lee cualquier proyecto (`memory_project_predicate!`, `crates/clawg-core/src/memory/search.rs`), así que compiten en todas las búsquedas.

**El arreglo que se agregó no alcanza.** project-registry dejó escrito que se colocarían con `clawg projects move --origin-project NOMBRE --to-project PROYECTO`, y el comando existe. Pero filtra por el proyecto que la nota tenía en su herramienta de origen, y las 1.594 notas no tienen ninguno: todas son de la sesión `manual-save`, sin proyecto ni directorio. Sólo 9 notas con proyecto de origen sin carpeta se resuelven con ese comando.

**Una reimportación no las mueve.** Una fila ya presente no cambia (`clawg import`, `docs/reference/commands.md`), así que ubicarlas por importación exige deshacer antes. `clawg import --undo 1 --dry-run` en prod dice que quitaría 54.165 filas y dejaría 271: 44 con veredicto y 227 piezas de un save con una fila que se queda. Deshacer pierde los accesos y el tier que esas filas ganaron desde entonces.

## Cómo ubicar las notas

El adaptador lee el campo `project` de cada observación del export, y `--project-map` ya coloca esos nombres. Por eso alcanza con un mapa de `sync_id` a proyecto, aplicado a un export recién hecho antes de importar, sin tocar el producto. El mapa no guarda texto; el export sí, con los secretos sin enmascarar, y se borra al terminar.

Cada señal se prueba primero contra las 2.345 notas cuyo proyecto se conoce. Lo medido el 2026-11-03:

| Señal | Aciertos | Contradice | Ubica, en cascada |
|---|---:|---:|---:|
| Título del save en una transcripción de la carpeta del proyecto | 659 | 4 | 51 |
| Primer segmento de la clave de tema, usado por un solo proyecto con 3 notas o más | 196 | 0 | 124 |
| Nombre de un proyecto en el texto | 364 | 149 | 125, no se acepta |

Con lo aceptado se ubican 175 de 1.594. Las transcripciones no llegan tan atrás: quedan 1.419 sin señal confiable. Faltan medir dos señales: las rutas que nombra la nota contra `git ls-files` de cada repositorio, y la cercanía en el tiempo con notas de proyecto conocido.

**Regla:** una señal se acepta con al menos 95 % de precisión sobre 30 casos decididos o más del control. Una nota con dos señales que discrepan no se ubica.

## Lo que no se pueda ubicar

Un proyecto sin carpeta, `historial-sin-ubicar`, creado con `clawg projects create`, y `--project-map NOMBRE=project:REF`. Esas filas dejan de competir en cada búsqueda y siguen a mano con `all_projects`. Se quedan sin proyecto sólo las preferencias y las notas de alcance personal, que valen en todos lados.

## Pasos

1. **Ensayo, en `dev`:** clasificar, deshacer e importar sobre una copia de prod, medir y dejar `dev` como estaba. Prod sólo recibe un `clawg backup`.
2. **Corte, en prod:** con el piloto aprobado, un `clawg backup`, un export nuevo, `clawg import --undo 1`, la importación con el mapa y la verificación.

## Cómo se mide

- **Tiempo:** segundos y filas por segundo contra la línea base, con el reparto por fase de `serve.log` y el máximo por minuto de `process.private_bytes` durante `embed`.
- **Ubicación:** filas sin proyecto antes y después, y cuántas fueron al proyecto de espera.
- **Integridad:** `clawg doctor` sin errores, y una segunda corrida con `--dry-run` que no agrega nada.
- **Las 271 filas que se quedan:** si una nota de esas queda partida entre dos proyectos. Si pasa, se colocan con `clawg projects move --id`.

## Por qué va en local

Necesita el store real, el export y las transcripciones de los clientes, un binario release y la misma máquina de la línea base.

## Decisiones abiertas

- [ ] Nombre y alcance del proyecto de espera.
- [ ] Si las filas siguen bajo el autor `migrated`. Por omisión sí: el workspace pide importancia 8 o más y tier 1, así que cambiar el autor aporta poco.
- [ ] Qué hacer con los archivos de la memoria nativa de Claude Code que `clawg doctor` cuenta sin importar: se decide con su `--dry-run`, en el corte.
