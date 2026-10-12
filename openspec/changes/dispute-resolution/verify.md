# dispute-resolution — Verificación

**Estado:** parcial. Los tests pasan en Linux y la hipótesis de los pedazos hermanos quedó confirmada con el modelo real. Falta correr en la máquina de Marcos lo que necesita el golden set y Windows (abajo, **Pendiente**).

## Tests

### Linux

`cargo test --workspace --no-fail-fast` en Linux.

| Corrida | Pasan | Fallan |
|---|---|---|
| Base (`origin/main`, 197186d) | 1.551 | 10 |
| Con el cambio (3cc85ac) | 1.588 | 10 |

Son 37 tests nuevos y ninguna falla nueva. Las 10 fallas son las mismas en las dos corridas y vienen del entorno:

- 7 de `doctor::tests`: modelan un PATH de Windows con `clawg.exe`.
- 3 de `diff::tests` en `rag-eval`: leen resultados de `docs/eval/results/`, que está en `.gitignore` y no existe en un clon limpio.

Cada commit del cambio se probó por separado, con el mismo comando: todos compilan y ninguno agrega fallas (1.569, 1.572, 1.575, 1.576, 1.578, 1.584 y 1.588 pasan; 10 fallan en todos).

### Windows

**Pendiente.** `cargo test --workspace --target-dir <otro-dir>` en la máquina de Marcos.

## Hipótesis de los pedazos hermanos (con el modelo real)

**Confirmada, y es mucho más grave de lo que suponía la exploración.**

Prueba hecha con multilingual-e5-small real (descargado a un directorio temporal, sin tocar `~/.clawg*`) a través de `EvalIngest`, que usa el mismo `memory_worker` que el daemon. Cada uno de los 40 `.md` de `openspec/changes/` se guardó como **un** save explícito `semantic_fact`, cada uno con su propio `agent_id`, así que el detector sólo podía emparejar pedazos del mismo save. El programa de la prueba no se commiteó.

| Corrida | Saves | Pedazos | Pedazos `disputed` | Saves con alguna disputa |
|---|---|---|---|---|
| `origin/main` (197186d) | 40 | 263 | 220 | 40 |
| Con el cambio (3cc85ac) | 40 | 263 | 0 | 0 |

En `main`, casi todos los pedazos después del primero quedan `disputed` contra sus hermanos: 220 de los 223 posibles (263 pedazos menos el primero de cada save). Los pedazos vecinos comparten el solapamiento del chunker y el mismo tema, y caen en la banda del detector. Con el cambio no queda ninguno.

Esto también importa para long-save-context, que esperaba esta respuesta: la causa común (pedazos de un mismo save que se tratan como afirmaciones independientes) queda confirmada, y la parte de las disputas queda resuelta acá.

## rag-eval

**Pendiente.** `docs/eval/golden.json` está fuera de git y no existe en el entorno donde se hizo el cambio. Hay que correr en la máquina de Marcos, compilando `rag-eval` en `origin/main` y en la rama:

| Corrida | Qué se espera |
|---|---|
| `rag-eval --real --diff` | Diff vacío en retrieval y similarity. La búsqueda no cambia: `superseded` ya estaba excluido y `disputed` puntúa igual que `active`. |
| `rag-eval --real --agent-view --diff` | Diff vacío en retrieval y similarity. En el bloque `agent_view` (que `--diff` no compara) pueden cambiar los tokens de `recall_context`: la nota de cada conflicto suma una línea, y el corpus que se ingiere por el pipeline real ya no genera disputas entre pedazos hermanos, así que puede haber menos conflictos que mostrar. Comparar los dos JSON campo por campo. |

La línea de base comprometida en la propuesta: R@1 0,4348, R@5 0,7826, R@10 0,8913, MRR 0,5979, nDCG@10 0,6693, sin bajar ninguna. Si se mide con el set generado (`golden-mine synth --seed 7`), comparar contra la corrida de `main` sobre el mismo archivo.

## Solapamiento con otros cambios en curso

- **storage (data-preservation B–E):** un undelete futuro devuelve filas cuyas disputas ya se liberaron al borrarlas (`delete_memory_core` ahora libera en su transacción). La disputa no vuelve sola. Está en los riesgos de la propuesta.
- **project-identity:** el detector compara dentro del mismo `project_id`. Un re-key cambia qué filas se ven entre sí, pero no toca este código.
- **lexical-fusion:** toca `search.rs`. Este cambio no lo modifica; el detector sigue con `lexical: None`.
- **context-density:** el bloque ACTIVE CONFLICTS ganó una línea (la que nombra `resolve_conflict`). Si ese cambio reescribe el bloque, tiene que conservar esa instrucción.
- **Migraciones:** este cambio no agrega ninguna, así que no compite por V9.

## Backlog del mapa

El comentario desactualizado de `worker.rs` que figura en el backlog de `openspec/README.md` quedó corregido en `fix(core): stop chunks of one save from disputing each other`. La línea del backlog se saca al archivar.
