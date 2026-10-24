# ram-footprint-gates — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | S (la medición); B, C y D, S–M cada uno y con puerta | 90, archivado (su paso 0 sale en 0.63.0); se coordina con 62, 73, 74, 85, 87 y 91 | exploración (borrador) |

**Tesis:** ram-footprint (90) sacó el refresco del índice de docs que corría en cada prompt (A) y dejó con qué atribuir cada pico de memoria (su paso 0). Quedan los brazos que dependen de una medición: despertar el modelo sólo al empezar una sesión (B), cargar el ONNX sin el `Vec` de 470 MB (C), embeber el import por tanda (D) y bajar el tiempo de ocio por defecto (E). Las alternativas, analizadas contra el código, están en la [exploración de 90](../archive/2026-10-24-ram-footprint/exploration.md#enfoques-posibles). Acá están la medición que las decide y sus puertas. Este cambio nació al partir 90 cuando se archivó.

## Paso 0: el día de uso real (sin código)

Se hace con autorización y en sólo lectura, en el perfil prod con el release 0.63.0, que es el primero con el paso 0 de 90. Después de un día de uso normal con Claude Code se lee:

- `clawg metrics report --by minute --since 1d` sobre `process.private_bytes`, `process.private_peak_bytes` (Windows) y `embedder.resident`. `--by version` separa las horas de 0.62.0, que todavía refrescaban los docs en cada prompt.
- En `~/.clawg/logs/serve.log`, cuatro tipos de línea. Las de carga: `embedding model loaded again in … for a wake`, `… for a call that embeds` o `… for a call that found its worker gone`, con la memoria privada antes y después. Las de primer uso: `the model a wake loaded was first used N s later`. Las de descarga: las que terminan en `; no call used it after the wake that loaded it`. Y las de refresco de docs.
- Si en el día hubo un import, sus líneas de fase con `private N MiB`.

Responde cinco preguntas:

1. **¿Cuánto tiempo pasa cargado el modelo con A?** Se compara contra el 73 a 81 % de los minutos medido en 0.62.0 (exploración de 90, «Lo medido en prod»).
2. **¿Cuántas cargas son de un `wake` que nadie usó?** Son las descargas marcadas sobre las cargas `for a wake`. Para las demás cargas de un `wake`, cuánto tardó el primer uso. Decide B.
3. **¿La carga es el máximo del proceso?** Se compara `process.private_peak_bytes` del minuto de cada carga con la memoria después de la carga y con los demás minutos. Decide C.
4. **¿Cuánto retiene el import por encima del modelo?** Se compara la memoria al terminar `embed` y `write` con la de `decide`. Decide D. Un día de uso normal puede no tener ningún import. En ese caso se mide con un import en el perfil dev, como import-timing (88), y se dice qué build se usó.
5. **¿Se mueve el piso?** Es la mínima de los minutos con `embedder.resident = 0` a lo largo del día. Si deriva, va a 91 o es un hallazgo nuevo. Si no, cierra la duda que dejó 90.

## Los brazos y sus puertas

| Brazo | Qué | Puerta | Decisión previa |
|---|---|---|---|
| B | Despertar el modelo sólo al empezar una sesión | Más de la mitad de las cargas de un día son de un `wake` que nadie usó (pregunta 2) | B1 (`user-prompt-submit` manda `wake: false`), B2 (sólo despierta el primer registro de una raíz) o B3 (ningún registro despierta y la primera búsqueda paga la carga) |
| C | Verificar el SHA-256 leyendo de a partes y cargar con `commit_from_file` en lugar de `commit_from_memory` | La carga es el máximo del proceso (pregunta 3) | Aceptar la ventana entre la verificación y la carga, con la misma confianza que ya se le da a `token` |
| D | Embeber y escribir el import por tanda, con import-job (87) | Los vectores y las filas preparadas pesan frente al modelo (pregunta 4) | Aceptar que un import cortado quede parcial y se pueda deshacer con `import --undo` |
| E | Bajar el `CLAWG_EMBEDDER_IDLE_MINUTES` por defecto | Después de B, si el modelo sigue cargado la mayor parte del día (pregunta 1) | Es una decisión de producto: cuánto espera la primera búsqueda después de una pausa |

Cada brazo que entre lleva un test que falla antes del cambio y repite la misma lectura después. Si la medición dice que ninguno vale la pena, el cambio cierra con lo medido, y ese es un resultado válido.

## Interacciones

- **ram-footprint (90, archivado).** Aportó A y el paso 0, especificados en `specs/project-docs`, `specs/metrics` (R6 y R8), `specs/storage` («Cada carga dice quién la pidió y cuánto costó») y `specs/portability` (IT1).
- **embedder-footprint (62).** Sus preguntas de memoria (a) y (b), el crecimiento con `IDLE_MINUTES=0` y el pico de la carga, se responden acá con las preguntas 3 y 5. A 62 le queda la CPU.
- **pipeline-metrics (73).** Lleva a `metrics.db`, de forma permanente, lo que las líneas de carga dejan en el log.
- **perf-bench (74).** Un escenario de memoria a ritmo fijo, leído con `--by minute`, es la forma repetible de este día.
- **embed-throughput (85).** Si adopta INT8, cambia cuánto ahorran B y C.
- **import-job (87).** D va con él o después.
- **tokenizer-footprint (91).** La pregunta 5 es su medición del piso en uso real.

## Decisiones abiertas

Son del usuario.

- [ ] **B.** ¿B1, B2 o B3? ¿El umbral es más de la mitad de las cargas de un día?
- [ ] **C.** ¿Se acepta la ventana entre la verificación y la carga?
- [ ] **D.** ¿Se acepta que un import que falla a mitad de camino quede parcial y se pueda deshacer, en lugar de no escribir nada?
- [ ] **E.** ¿Se considera después de B?

## Riesgos

- **Un día puede no ser representativo.** Si fue liviano, sin pausas largas o sin búsquedas, cambian las proporciones de la pregunta 2. Por eso se dice cuántas sesiones y prompts tuvo, y si hace falta se mide otro día.
- **B hace esperar a la primera búsqueda después de una pausa.** El tiempo de una carga en release no está medido; las líneas de carga del mismo día lo miden.
- **C abre una ventana entre la verificación y la carga, y D cambia una garantía del import.** Por eso los dos van con una decisión explícita.
