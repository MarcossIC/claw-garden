# docs-search-quality — Verificación

Mediciones con el modelo real (`multilingual-e5-small` de `~/.clawg-dev/models`), en Windows y con un build debug: el tiempo de indexado es el de ese build y no se compara con uno release. Desde `dq-base` el indexado lee los vectores que no cambiaron del memo de `docs/eval/cache/` y sólo embebe el resto, así que su tiempo no se compara con el de las corridas sin memo.

## Línea base (tarea 4)

### Antes del filtro (`9bcc2ed`, el commit anterior a la tarea 1)

Sin archivo de resultados (la tarea 2 no existía): anotado de stdout el 2026-10-30, desde un worktree descartable.

- `indexed 727 docs into 17584 passages (2 skipped) in 2407.1 s`; ningún doc dejado fuera (el filtro no existía).

| | Hit@1 | Hit@3 | Hit@5 | MRR@10 | preguntas |
|---|---|---|---|---|---|
| todas | 0,13 | 0,20 | 0,20 | 0,19 | 15 |
| docs | 0,10 | 0,20 | 0,20 | 0,17 | 10 |
| código | 0,20 | 0,20 | 0,20 | 0,23 | 5 |

Por pregunta (puesto, `-` fuera de los 10 primeros): d01 2, d02 -, d03 1, d04 -, d05 6, d06 -, d07 -, d08 -, d09 -, d10 -, c01 -, c02 1, c03 -, c04 6, c05 -.

El primer hit de 6 de las 15 (d01, d02, d05, d07, c03 y c05) es `openspec/changes/archive/2026-10-15-project-indexing/verify.md > … > Golden set de docs`, el doc que copia las preguntas: es el sesgo que la tarea 1 saca del corpus. En d01 la respuesta quedó segunda, detrás de esa copia.

d09 no podía acertar en ninguna corrida: su heading esperado ya no existía (tarea 5).

### Con el filtro (`0f09b2b`, `2026-10-31-real-docs-cost-d.json`)

Se toma esta corrida en lugar de una de `5a44fdf`: la de `5a44fdf` se cortó durante el build porque el sistema se quedó sin memoria, y `0f09b2b` ya tiene el filtro (tarea 1), el archivo de resultados (tarea 2) y el golden de 40 (tarea 5). Es la corrida `cost-d` de docs-index-cost, sin memo.

- `indexed 740 docs into 18249 passages (2 skipped) in 1601.5 s`; `left out 1 doc quoting a question: openspec/changes/archive/2026-10-15-project-indexing/verify.md`.

Sobre las 15 preguntas de antes. No es exactamente la misma medición: d05, d09 y c03 se corrigieron en el medio (tarea 5).

| | Hit@1 | Hit@3 | Hit@5 | MRR@10 | preguntas |
|---|---|---|---|---|---|
| todas | 0,20 | 0,20 | 0,20 | 0,22 | 15 |
| docs | 0,20 | 0,20 | 0,20 | 0,22 | 10 |
| código | 0,20 | 0,20 | 0,20 | 0,23 | 5 |

Por pregunta: d01 1, d03 1, d05 6, c02 1, c04 6, y las demás fuera de los 10 primeros. d01 pasa del puesto 2 al 1: la copia que tenía delante quedó fuera del corpus.

Sobre las 40 del golden de entonces:

| | Hit@1 | Hit@3 | Hit@5 | MRR@10 | preguntas |
|---|---|---|---|---|---|
| todas | 0,075 | 0,100 | 0,100 | 0,098 | 40 |
| docs | 0,100 | 0,100 | 0,100 | 0,113 | 20 |
| código | 0,050 | 0,100 | 0,100 | 0,083 | 20 |

## Línea base del paso 2 (`cabfaa8`, `2026-10-31-real-docs-dq-base.json`)

El golden de hoy: 40 casos, sha256 `d7a573b88115c6ea…`. Desde `cost-d`, d05 y d15 esperan `docs/reference/` (la referencia salió de `.claude/CLAUDE.md`), d16 se fue porque sólo la respondían archivos de instrucciones para agentes, y d21 la reemplaza con un id nuevo.

- 750 docs, 18457 pasajes, 2 salteados, el mismo doc dejado fuera; indexado en 124,2 s, con 17775 vectores del memo y 682 embebidos.

| | Hit@1 | Hit@3 | Hit@5 | MRR@10 | preguntas |
|---|---|---|---|---|---|
| todas | 0,075 | 0,100 | 0,100 | 0,091 | 40 |
| docs | 0,100 | 0,100 | 0,100 | 0,100 | 20 |
| código | 0,050 | 0,100 | 0,100 | 0,082 | 20 |

Por pregunta: d01 1, d03 1, c02 1, c14 2, c04 7, y las demás fuera de los 10 primeros. Contra `cost-d`, d05 sale de los 10 (estaba en el 6), y su primer hit es la sección «Data profiles» que todavía tiene `.claude/CLAUDE.md`. c04 pasa del 6 al 7.

## Paso 2: expresión léxica de docs

### 2a. Palabras vacías y tokens cortos (`875d7b0`, revertido en `51c3a6b`): no pasa la puerta

`2026-10-31-real-docs-dq-2a.json`: el mismo golden; 751 docs y 18469 pasajes (lo que suma el código de 2a), el mismo doc dejado fuera; indexado en 74,2 s, con 18437 vectores del memo y 32 embebidos.

| | Hit@1 | Hit@3 | Hit@5 | MRR@10 | preguntas |
|---|---|---|---|---|---|
| todas | 0,025 | 0,075 | 0,150 | 0,062 | 40 |
| docs | 0,000 | 0,100 | 0,150 | 0,052 | 20 |
| código | 0,050 | 0,050 | 0,150 | 0,073 | 20 |

`rag-eval --diff dq-base dq-2a`, sobre todas las preguntas:

| | mejor | peor | neto | p |
|---|---|---|---|---|
| Hit@1 | 0 | 2 | −2 | 0,50 |
| Hit@3 | 1 | 2 | −1 | 1,00 |
| Hit@5 | 3 | 1 | +2 | 0,63 |
| MRR@10 | 3 | 3 | 0 | 1,00; media −0,029 [−0,102, +0,028] |

Por pregunta: d01 1 → fuera de los 10, d02 fuera → 2, d03 1 → 3, d05 fuera → 5, c04 7 → 4 y c14 2 → 5; las demás no se mueven.

La puerta pide que ninguna pregunta del top 3 salga de él y que Hit@3 gane al menos 2. Salen d01 y c14 y el neto es −1, así que 2a se revirtió.

### Línea base de 2b y 2c (`ebd99fb`, `2026-10-31-real-docs-dq-base2.json`)

Entre `cabfaa8` y `ebd99fb` entró el merge del fallback de hilos del embedder, que suma docs sobre su RAM. Para que esos docs no se mezclen con el efecto de cada pieza, 2b y 2c se miden contra esta corrida. 753 docs y 18480 pasajes; indexado en 77,3 s, con 18417 vectores del memo y 63 embebidos. Da exactamente lo mismo que `dq-base`, pregunta por pregunta: el merge no movió nada.

### 2b. Orden por rareza (`459f024`, revertido en `86737e0`): no pasa la puerta

Sin lista de vacías y sin quitar repetidos: el tope de 16 se queda con los tokens que el orden pone primero (identificadores, después los más largos, desempate por posición), en lugar de con los 16 primeros. 30 de las 40 preguntas tienen más de 16 tokens. `2026-10-31-real-docs-dq-2b.json`: 754 docs y 18484 pasajes, indexado en 71,5 s.

| | Hit@1 | Hit@3 | Hit@5 | MRR@10 | preguntas |
|---|---|---|---|---|---|
| todas | 0,050 | 0,075 | 0,075 | 0,062 | 40 |
| docs | 0,050 | 0,050 | 0,050 | 0,050 | 20 |
| código | 0,050 | 0,100 | 0,100 | 0,074 | 20 |

`--diff dq-base2 dq-2b`: Hit@1, Hit@3 y Hit@5 dan 0 mejor y 1 peor; MRR@10 0 mejor y 2 peor, con media −0,029 [−0,083, +0,000]. d01 pasa del 1 a fuera de los 10 y c14 del 2 al 3. d01 sale del top 3, así que 2b se revirtió. Su primer hit vuelve a ser `openspec/changes/archive/2026-10-21-query-plans/verify.md > … > Linux (2026-10-21)`, como en 2a: el orden rescata las últimas palabras de la pregunta, que el tope de hoy corta, y esas palabras llevan a esa bitácora.

### 2c. AND de los 3 más raros, con respaldo OR (`f0479d4`, revertido en `ff709e0`): no pasa la puerta

Medida sin 2b: el orden por rareza sólo elige los 3 tokens del AND, y la vuelta al OR es la expresión de hoy, sin cambios. `2026-10-31-real-docs-dq-2c.json`: 754 docs y 18486 pasajes, indexado en 72,6 s.

`--diff dq-base2 dq-2c`: ninguna pregunta se mueve, en ninguna métrica. Cambia el primer hit de d21 y el de c11, que siguen sin acertar. Neto 0 en Hit@3 contra los +2 que pide la puerta, así que 2c se revirtió.

### Por qué el paso 2 no puede pasar su puerta con este golden

Cada pregunta del golden se escribió sin las palabras del pasaje que la responde (tarea 5): con su respuesta comparte, a lo sumo, palabras vacías. Así, el canal léxico no puede encontrar la respuesta de ninguna pregunta por sus palabras de contenido, y una expresión distinta sólo cambia qué otros pasajes propone:

- 2a quitó lo único que la respuesta compartía con la pregunta.
- 2b sumó palabras del final de la pregunta, que llevan a otros pasajes.
- 2c casi nunca encontró las 3 palabras juntas.
- 2e no tiene sobre qué actuar: ninguna pregunta nombra una ruta ni un archivo.
- 2d cambia las palabras que entran en el tope, como 2b, en 18 preguntas.

Lo que el golden mide es el canal denso. Medir el léxico necesita preguntas que nombren lo que el pasaje dice (un identificador, una ruta, un término propio), y este golden las excluye por construcción. Sin las palabras vacías, el canal léxico favorece pasajes densos en palabras de contenido sueltas por encima de la respuesta. El primer hit de d01 pasa a ser `openspec/changes/archive/2026-10-21-query-plans/verify.md > … > Linux (2026-10-21)`, el de d02 `crates/clawg-cli/src/setup.rs > mod tests` y el de c14 `crates/clawg-cli/src/daemon/metrics.rs > mod tests > fn each_minute_and_the_stop_are_written()`.
