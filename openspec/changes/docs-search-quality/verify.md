# docs-search-quality — Verificación

Mediciones con el modelo real (`multilingual-e5-small` de `~/.clawg-dev/models`), en Windows y con un build debug: el tiempo de indexado es el de ese build y no se compara con uno release.

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

### Con el filtro (`5a44fdf`, la tarea 3)

Pendiente: la corrida se cortó durante el build porque el sistema se quedó sin memoria.
