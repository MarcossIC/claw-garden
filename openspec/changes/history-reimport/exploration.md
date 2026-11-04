# history-reimport — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 y 2 | M, operativo, sin código del producto | — | ensayo hecho ([verify.md](verify.md)); el corte espera la aprobación del piloto |

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

Con lo aceptado se ubicaban 175 de 1.594, y las transcripciones no llegan tan atrás. El ensayo midió las dos señales que faltaban y corrigió dos cifras de esta tabla ([verify.md](verify.md)):

- **Las rutas** que nombra la nota contra `git ls-files` de cada repositorio se aceptan: 1.086 aciertos y 7 contradicciones.
- **La cercanía en el tiempo** se rechaza. Da más de 95 % en el control, pero sobre las notas sin proyecto coincide con las otras señales cerca de la mitad de las veces.
- **La clave de tema** da 172 sin contradicciones al dejar afuera la nota evaluada: las 196 eran circulares.
- **Las transcripciones** dan 628 sin contradicciones, con alcance 33, si sólo cuenta la llamada a `mem_save` cuyo título aparece en una sola carpeta.

Con esto se ubican 834 de las 1.597 notas sin proyecto, contando 3 prompts. 684 de esas 834 dependen sólo de las rutas.

**Regla:** una señal se acepta con al menos 95 % de precisión sobre 30 casos decididos o más del control, y debe sostenerse sobre las notas sin proyecto contra las señales que no comparten su fuente. Una nota con dos señales que discrepan no se ubica.

## Lo que no se pueda ubicar

Un proyecto sin carpeta, `historial-sin-ubicar`, creado con `clawg projects create`, y `--project-map NOMBRE=project:REF`. Esas filas dejan de competir en cada búsqueda y siguen a mano con `all_projects`. Se quedan sin proyecto sólo las preferencias y las notas de alcance personal, que valen en todos lados.

En el ensayo fueron al proyecto de espera 735 notas sin señal o con señales que discrepan, más las 9 de proyectos de origen sin carpeta: 6.311 filas. Se quedaron sin proyecto 28 notas.

## Pasos

1. **Ensayo, en `dev`:** hecho. Clasificó, deshizo e importó sobre una copia de prod, midió y dejó `dev` con su contenido. Prod sólo recibió un `clawg backup`.
2. **Corte, en prod:** con el piloto aprobado, un `clawg backup`, un export nuevo, clasificar las notas que el mapa no conoce, `clawg projects create`, el `--dry-run` con el mapa, `clawg import --undo 1`, la importación con el mapa, colocar con `clawg projects move --id` las filas retenidas que el mapa ubica, y la verificación.

**Antes del corte:** cada snapshot automático puede empujar afuera otro, porque se conservan 5 más el último de cada motivo (`AUTO_KEEP`, `crates/clawg-core/src/memory/backup.rs`). En el ensayo eso costó 4 snapshots viejos de `dev`. En prod caerían 4, incluido el `pre-import` de la primera importación si se hacen los 5 `projects move` ([verify.md](verify.md)). Los `manual` no se podan.

## Cómo se mide

- **Tiempo:** segundos y filas por segundo contra la línea base, con el reparto por fase de `serve.log` y el máximo por minuto de `process.private_bytes` durante `embed`.
- **Ubicación:** filas sin proyecto antes y después, y cuántas fueron al proyecto de espera.
- **Integridad:** `clawg doctor` sin errores, y una segunda corrida con `--dry-run` que no agrega nada.
- **Las 271 filas que se quedan:** si una nota de esas queda partida entre dos proyectos. En el ensayo ninguna quedó partida: la importación encuentra la nota por su origen y no la toca. El costo es que 18 notas, con 90 filas, siguen sin proyecto aunque el mapa las ubique; `clawg projects move --id` las coloca, comprobado con `--dry-run`.

## Por qué va en local

Necesita el store real, el export y las transcripciones de los clientes, un binario release y la misma máquina de la línea base.

## Decisiones abiertas

- [ ] Aprobar el piloto y hacer el corte.
- [ ] Si alcanza con que 684 notas se ubiquen sólo por las rutas: unas 25 quedarían mal, hasta unas 69 en el límite inferior medido.
- [ ] Si se aceptan las 27 notas que quedan sin fila activa porque otra de su proyecto comparte su clave de tema.
- [ ] Si se copia aparte algún snapshot automático de prod antes de que el corte lo empuje afuera.
- [ ] Nombre y alcance del proyecto de espera.
- [ ] Si las filas siguen bajo el autor `migrated`. Por omisión sí: el workspace pide importancia 8 o más y tier 1, así que cambiar el autor aporta poco.
- [ ] Qué hacer con los archivos de la memoria nativa de Claude Code que `clawg doctor` cuenta sin importar: se decide con su `--dry-run`, en el corte.
