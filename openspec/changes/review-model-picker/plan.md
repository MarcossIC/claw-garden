# review-model-picker — Plan

Corre en: local · Necesita a la persona: para correr la prueba a mano en una terminal real

## Problema

`clawg preferences set review-model` sin `MODEL` imprime todos los modelos del CLI de Cursor en una lista numerada y pide un número o un id (`crates/clawg-cli/src/review.rs:425-438`, `prefs::numbered` en `crates/clawg-cli/src/review/prefs.rs:256-262`, `prefs::choose` en `:147-154`). La lista que imprime `agent --list-models` el 2026-11-26 tiene 284 modelos: hay que leerla entera, desplazar la terminal y tipear un número de tres cifras.

Por familia, esa lista tiene: Claude 133, GPT 75, Grok 22, Gemini 20, Muse 12, Codex 8, DeepSeek 5, Kimi 4, GLM 2, Composer 2 y Auto 1. Una familia sola no alcanza para que la lista entre en una pantalla: dentro de Claude hay 15 versiones (Opus 5.5, Fable 5.2, Sonnet 5…) y la versión con más variantes de esfuerzo y velocidad tiene 20.

## Objetivo y fuera de alcance

Sirve al objetivo 4, por el flujo de revisión: elegir el modelo de `clawg review` sin leer 284 líneas.

Cambia sólo lo que pasa con una terminal y sin `MODEL`: en lugar de la lista numerada y el número, una pantalla que se recorre con las flechas, en tres pasos como mucho (familia, versión, modelo), donde un modelo se marca con Enter o Espacio y se confirma con una segunda pulsación sobre el marcado.

Fuera de alcance:

- `MODEL` dado como argumento, `--project`, `--cursor`, `unset` y el formato de `review.json` no cambian.
- Sin terminal en la entrada sigue como hoy: la lista numerada, salida 2 y nada escrito (`crates/clawg-cli/src/review.rs:429-432`).
- No se agrega filtro por texto, ni colores, ni se cambia `clawg review --model`.
- No se cambia cómo se lee la lista del CLI (`prefs::parse_model_list`, `crates/clawg-cli/src/review/prefs.rs:135-144`).

## Hecho

Agrupar, en `crates/clawg-cli/src/review/picker.rs` (módulo nuevo):

- H1. Dado un modelo listado, cuando se agrupa, entonces su familia MUST llamarse como las letras con que empieza su nombre (`GPT-5.5 1M` → `GPT`, `Codex 5.3 High` → `Codex`, `Grok 4.5` de `cursor-grok-4.5-high` → `Grok`), y como la primera palabra de su id cuando el nombre no empieza con una letra → `picker::tests::a_family_is_named_by_the_letters_its_model_names_start_with`
- H2. Dados dos modelos de una familia cuyos ids sólo difieren en las palabras finales de esfuerzo y velocidad (`none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `extra`, `max`, `fast`, `thinking`), cuando se agrupa, entonces MUST quedar en la misma versión; un id nunca se queda sin palabras → `picker::tests::a_version_holds_the_ids_that_differ_only_in_effort_and_speed`
- H3. Dada una versión, entonces su etiqueta MUST ser las palabras iniciales que comparten los nombres de sus modelos (`Claude Opus 5.5 1M`), y el nombre entero cuando tiene un solo modelo → `picker::tests::a_version_is_labelled_by_the_words_its_names_share`
- H4. Dada una lista, cuando se agrupa, entonces cada modelo MUST estar en una versión y sólo en una, y familias, versiones y modelos MUST conservar el orden en que el CLI los lista → `picker::tests::every_listed_model_is_in_one_version_in_the_order_listed`

Recorrer, en el mismo módulo:

- H5. Dado un modelo vigente que está en la lista, cuando se abre el selector, entonces MUST empezar en las familias con el cursor sobre la del modelo vigente; cuando no está, sobre la primera → `picker::tests::the_picker_opens_on_the_family_in_use`
- H6. Dada cualquier pantalla, cuando se pulsa ↑ o ↓, entonces el cursor MUST moverse una fila y dar la vuelta en los extremos → `picker::tests::the_cursor_moves_and_wraps`
- H7. Dada una familia con más de una versión, cuando se la elige, entonces MUST abrirse la pantalla de sus versiones; con una sola versión MUST abrirse directo la de sus modelos → `picker::tests::a_family_opens_its_versions_or_its_only_versions_models`
- H8. Dada la pantalla de modelos, cuando se pulsa Enter o Espacio sobre uno sin marcar, entonces MUST quedar marcado sin elegirse; cuando se pulsa otra vez sobre el marcado, MUST elegirse ese id → `picker::tests::a_model_is_marked_first_and_chosen_on_the_second_press`
- H9. Dado un modelo marcado, cuando se pulsa Enter o Espacio sobre otro, entonces la marca MUST pasar a ése sin elegir ninguno → `picker::tests::marking_another_model_moves_the_mark`
- H10. Dada la pantalla de modelos o de versiones, cuando se vuelve atrás, entonces MUST mostrarse el paso anterior sin marca; en las familias, volver atrás MUST cancelar → `picker::tests::back_leaves_one_step_and_cancels_at_the_families`
- H11. Dada cualquier pantalla, cuando se pulsa Ctrl-C o `q`, entonces MUST cancelar sin elegir → `picker::tests::quitting_cancels_from_any_screen`

Dibujar, en el mismo módulo:

- H12. Dada la pantalla de familias, entonces la fila del cursor MUST ir resaltada, la familia del modelo vigente en intensidad normal y con la palabra de «en uso», y las demás atenuadas → `picker::tests::the_family_in_use_is_brighter_and_the_cursor_brightest`
- H13. Dada la pantalla de modelos, entonces cada fila MUST llevar `○`, o `●` la marcada, y el nombre y el id del modelo → `picker::tests::each_model_has_a_circle_filled_when_marked`
- H14. Dada la pantalla de modelos, entonces la ayuda MUST decir, en español y en inglés, que la primera pulsación marca y que otra sobre el marcado confirma → `picker::tests::the_help_says_the_second_press_confirms`
- H15. Dada una terminal de `filas` × `columnas`, entonces el cuadro MUST tener como mucho `filas` líneas y ninguna más ancha que `columnas`, con la fila del cursor siempre a la vista y cuántas filas quedan arriba y abajo cuando no entran → `picker::tests::a_frame_fits_the_terminal_and_keeps_the_cursor_in_view`

Teclas y terminal:

- H16. Dados los bytes que entrega una terminal, cuando se decodifican, entonces `ESC [ A`/`ESC O A` MUST ser arriba, `ESC [ B`/`ESC O B` abajo, `\r`, `\n` y espacio elegir, `ESC` solo, `ESC [ D`, `0x7f` y `0x08` volver, `0x03` y `q` cancelar, y cualquier otra secuencia MUST ignorarse → `picker::tests::keys_are_read_from_the_bytes_a_terminal_sends`
- H17. Dada una terminal que entrega una serie de teclas, cuando corre el selector sobre ella, entonces MUST dibujar un cuadro por tecla y devolver el id elegido, o la cancelación → `picker::tests::the_picker_runs_over_a_terminal_until_a_choice_or_a_cancel`
- H18. En Windows, dado el modo de la consola, entonces el de entrada MUST quedar sin línea, sin eco y sin Ctrl-C procesado y con entrada de terminal virtual, y el de salida con secuencias de terminal virtual → `terminal::tests::the_console_modes_for_the_picker` (sólo Windows)
- H19. En Unix, dado un `termios`, entonces el crudo MUST quedar sin `ICANON`, `ECHO` ni `ISIG`, con `VMIN` 1 y `VTIME` 0 → `terminal::tests::the_termios_for_the_picker` (sólo Unix)

El comando:

- H20. Dadas la entrada y la salida, entonces el selector MUST usarse sólo con las dos en una terminal; con la entrada en una terminal y la salida no, MUST pedirse el número como hoy; sin terminal en la entrada, nada → `review::tests::the_picker_needs_both_ends_on_a_terminal`
- H21. Sin terminal, `set review-model` sin `MODEL` MUST seguir listando numerado y saliendo con 2 → `choosing_a_review_model_without_a_terminal_lists_them_and_exits_2` (`crates/clawg-cli/tests/binary.rs:1278`), sin tocarlo
- H22. Dada una cancelación, entonces el comando MUST decir que no se cambió nada, salir con 0 y dejar `review.json` como estaba → `review::tests::a_cancelled_choice_says_nothing_changed` (la frase, en los dos idiomas) y la prueba a mano de abajo

No cambia:

- Nada fuera de `crates/clawg-cli/src/review.rs`, `crates/clawg-cli/src/review/prefs.rs`, los dos módulos nuevos `crates/clawg-cli/src/review/picker.rs` y `crates/clawg-cli/src/review/terminal.rs`, `crates/clawg-cli/Cargo.toml` (una feature de `windows-sys`; `Cargo.lock` no cambia), `openspec/specs/preferences/spec.md` y `docs/reference/commands.md`.
- No se afloja ni se borra ningún test existente.

Lo que ningún test alcanza, porque necesita una consola real: que el modo crudo se ponga y se restaure, y que las flechas lleguen como secuencias. Queda como prueba a mano, bajo Verificación.

## Decisión

**Tres pasos donde hacen falta.** Familia → versión → modelo, y el paso de versión se salta en las familias de una sola versión. Alternativas: dos pasos con una ventana que se desplaza, que en Claude deja 133 filas por recorrer; dos pasos con filtro al tipear, que suma un modo de entrada de texto y más teclas que explicar. Con tres pasos ninguna pantalla pasa de 20 filas en la lista de hoy.

**Agrupar por regla, no por tabla.** La familia sale del nombre y la versión del id, sin una lista de familias conocidas: el CLI de Cursor cambia sus modelos en cada versión y una tabla quedaría vieja. Las filas muestran los nombres enteros como los da el CLI: `Claude Opus 5.5 1M` es el de esfuerzo medio y no tiene una palabra propia que lo distinga, así que recortarlos dejaría una fila vacía.

**Código propio sobre `windows-sys` y `libc`, sin crossterm.** Los dos ya se compilan (`crates/clawg-cli/Cargo.toml:73` y `:92`), así que no entra ningún crate. crossterm 0.29.0 con `events` y `windows`, resuelto contra este `Cargo.lock`, suma seis: crossterm, crossterm_winapi, document-features, litrs, signal-hook y signal-hook-mio. A cambio, el lado de Unix se compila y se testea en CI pero no se prueba a mano en una consola.

**Entrada de terminal virtual en Windows.** Con `ENABLE_VIRTUAL_TERMINAL_INPUT` la consola entrega las flechas como las mismas secuencias que una terminal de Unix, y un solo decodificador puro sirve a los dos. La alternativa, `ReadConsoleInputW`, pide un decodificador por sistema. Si en el paso 5 la lectura estándar no entrega las secuencias en una consola real, se pasa a `ReadConsoleInputW` y Registro lo anota.

**Pantalla alterna.** El selector se dibuja en la pantalla alterna de la terminal (`ESC [ ? 1049 h`) y al salir vuelve lo que había: redibujar es borrar y escribir, sin contar líneas. La alternativa, redibujar en el lugar, se rompe cuando una línea se parte por el ancho.

**Sin colores.** Sólo atenuado, normal, negrita e invertido, que se leen en cualquier tema; el puntero `›`, los círculos y la palabra «en uso» dicen lo mismo sin estilo. Los signos (`○ ● › ↑ ↓ ·`) están en las fuentes de consola de Windows.

**Cancelar sale con 0.** No cambiar nada es lo que la persona eligió, no un error; la frase dice que no se cambió nada.

**Cuando el modo crudo no se puede poner** (una terminal que no es consola de Windows, como mintty sin winpty), se pide el número como hoy, en vez de fallar.

## Pasos

1. `picker::tests` de H1 a H4 → `families()` y sus tipos en `review/picker.rs`. `feat(review): group the Cursor CLI's models by family and version`
2. `picker::tests` de H5 a H11 → `Picker`, `Key`, `Outcome` y `press`. `feat(review): walk the model picker with keys`
3. `picker::tests` de H12 a H15 → el cuadro, la ventana y las frases nuevas en `prefs::Said` (con su caso en `every_sentence_is_in_both_languages`). `feat(review): draw the model picker`
4. `picker::tests` de H16 y H17 → `decode`, el trait de la terminal y el bucle que corre el selector sobre ella. `feat(review): run the model picker over a terminal`
5. `terminal::tests` de H18 y H19, `review::tests` de H20 y H22 → `review/terminal.rs` (modo crudo, tamaño, lectura y dibujo, restaurados al soltar), la feature `Win32_System_Console`, el selector en `set_model`, y en el mismo commit RM1 de `openspec/specs/preferences/spec.md` y `docs/reference/commands.md`. `feat(review): choose the review model with the arrow keys`

## Registro

- 2026-11-26. Tamaño grande: había más de un enfoque. Las dos decisiones con alternativas (los pasos y crossterm) se le plantearon a la persona antes de escribir el plan.
- 2026-11-26. Plan aprobado por la persona.
- Pasos 1 a 5 hechos, un commit cada uno: `e388002f`, `2c239b03`, `47d4269c`, `10354125`, `f317c1f0`. Un sexto, `dbd8025b`, saca de `families` un tipo anidado que clippy rechazaba (`type_complexity`); no cambia lo que hace.
- La regla de agrupar se corrió contra la lista real del CLI (284 modelos) con un test temporal que no se commiteó: 11 familias, 15 versiones en Claude, 20 modelos en la versión más grande.
- Más de lo que pedía Hecho: al entrar a la familia y a la versión vigentes, el cursor arranca sobre la versión y el modelo vigentes (H5 sólo pedía la familia); `Q` cancela igual que `q`; `\r\n` en una misma lectura cuenta como una sola pulsación, para que un Enter no marque y confirme; una flecha con parámetros (Ctrl-↑, por ejemplo) se ignora. La pintura del cuadro tiene su test, `a_frame_is_painted_from_the_top_with_its_tones`, que Hecho no nombraba.
- Las frases del selector quedaron en `prefs::Said`, como decía el paso 3; la de la cancelación (`NotChosen`) entró en el paso 5, con su uso.
- Desvío: `ICRNL`, `IXON` e `IEXTEN` no se tocan en Unix. El decodificador ya lee `\n` como Enter, y H19 sólo pide `ICANON`, `ECHO` e `ISIG`.
- Sin comprobar todavía: el lado de Unix de `review/terminal.rs` no se compiló en local (no hay toolchain de Linux en la máquina donde se implementó); lo compila y lo testea el CI. Sus llamadas se contrastaron con las definiciones de `libc` 0.2.184 para `x86_64-unknown-linux-gnu`.
- Sin comprobar todavía: el selector sobre una consola real. Un intento de manejarlo con teclas inyectadas en una consola oculta se abandonó; queda la prueba a mano de Verificación.
- Desvío de H17: el selector dibuja un cuadro por lectura, no por tecla. Varias teclas de una misma lectura dan un solo cuadro, porque de los intermedios sólo se vería el último, y una lectura sin ninguna tecla del selector dibuja otra vez. Un cambio de tamaño de la terminal se ve recién con la tecla siguiente: el tamaño se lee antes de cada cuadro, y nada despierta la lectura cuando cambia.
- Desvío de H15: con menos de tres líneas no entran la fila del cursor y las dos de cuántas quedan; ahí van las filas solas. Quedó escrito en RM2.
- Después de la primera ronda de revisión: `conclude` reúne en `review.rs` el final de una elección, para que el test de la cancelación pruebe también que `review.json` queda como estaba; `picker` y `terminal` pasaron a ser módulos privados de `review`; las dos frases nuevas con imperativo quedaron de tú, como pide CL3.
- `ESC O D` vuelve un paso igual que `ESC [ D`: es la flecha izquierda de una terminal en modo de aplicación. H16 sólo nombraba la segunda.

## Verificación

### Lo que se corrió

- `cargo test --workspace --target-dir target-test`, en Windows, sobre `9e969bde`: salida 0, y cada binario de tests termina en `ok`.
- `cargo clippy --locked --workspace --all-targets --target-dir target-test -- -D warnings` y `cargo clippy --locked -p clawg-cli --target-dir target-test -- -D warnings`, sobre `9e969bde`: sin hallazgos.
- La regla de agrupar contra la lista real del CLI de Cursor del 2026-11-26, con un test temporal: 284 modelos en 11 familias, 15 versiones en Claude y 20 modelos en la versión más grande.

### Revisión, ronda 1 (sobre `bc279703..7636ba67`)

`clawg review` (sesión `4915209e-1b4b-4b8a-945d-9e5495261cd0`) terminó en `BLOCKED` y `change-reviewer` en `WARNINGS`. Los dos coinciden en que H1 a H14, H16 a H18, H20 y H21 se cumplen, y en lo que sigue.

| Hallazgo | Quién | Qué se hizo |
|---|---|---|
| H22: la salida 0 y `review.json` intacto tras cancelar sólo se leían en el código | los dos | `conclude` reúne el final de una elección y `a_cancelled_choice_says_nothing_changed` prueba que una cancelación devuelve `Ok` y deja el archivo ausente o idéntico byte a byte (`9e969bde`). La corrida en una consola real sigue pendiente, abajo |
| El selector no corrió en una consola real | los dos | pendiente en local, abajo |
| H15: en una terminal baja no se decía cuántas filas quedan | los dos | arreglado: sin lugar para el título, van las filas con sus dos líneas de cuántas quedan; con menos de tres líneas, las filas solas. Con test para 8, 3 y 2 líneas |
| H19: el lado de Unix no se compiló ni corrió | los dos | pendiente del CI, abajo |
| `decode`: un Esc se llevaba el Esc o el Ctrl-C siguiente, y la cola de `ESC ESC [ 2 q` se leía como Enter y `q` | `clawg review` | confirmado contra el código y arreglado, con los tres casos en `keys_are_read_from_the_bytes_a_terminal_sends` |
| `picker` y `terminal` eran módulos públicos sin uso fuera de `review` | `clawg review` | privados. En Unix, `raw` exige también que la salida sea una terminal, como ya hacía en Windows |
| «Elegí una familia» y «elegí una versión» en voseo, contra CL3 | `clawg review` | pasadas a tú |
| Una lectura interrumpida por una señal cortaba el comando; el comentario de `libc` en `Cargo.toml` no nombraba `review/terminal.rs` | `clawg review`; el comentario, los dos | la lectura interrumpida no entrega teclas y se redibuja; comentario al día |
| El test del orden no afirmaba el de las familias entre sí | `clawg review` | agregado |
| H17: un cuadro por lectura, no por tecla | `change-reviewer` | confirmado; es lo que se quería y quedó como desvío en Registro |
| Un Esc partido de su secuencia por una terminal lenta se lee como Esc | `change-reviewer` | confirmado; no se arregla en este cambio y quedó como límite conocido en RM2: vuelve un paso o cancela sin cambiar nada |
| El ancho se cuenta en caracteres, no en columnas | `change-reviewer` | límite conocido en RM2 |
| Ningún test cubre la caída al número tipeado cuando la terminal no se deja leer tecla por tecla | `change-reviewer` | sin terminal no hay cómo provocarlo en un test; entra en la prueba a mano |
| Anteriores a este cambio: `Said::NeedsModel` y `missing_cursor` en voseo; una línea vacía en el número tipeado sale con 1 y un mensaje que empieza con un espacio | `clawg review` | no se tocan en este cambio |

### Revisión, ronda 2 (sobre `7636ba67..84342a31`)

`clawg review` (la misma sesión, con `--resume`) y `change-reviewer` terminaron en `WARNINGS`. Los dos dan por resueltos los hallazgos de la ronda 1 que se arreglaron, sin ningún test aflojado, y dejan abiertas sólo las dos comprobaciones de abajo. `change-reviewer` sumó dos cosas:

- `9e969bde` junta varios arreglos en un commit, contra la regla de un cambio coherente por commit. No se pierde nada; partirlo es reescribir la historia, y no se hizo.
- El comentario de `RawTerminal::read` y una línea de Registro decían que una lectura interrumpida recoge un cambio de tamaño. No es así: nada despierta la lectura cuando la terminal cambia de tamaño. Corregidos los dos textos; el código no cambia.

### Pendiente en local

El cambio no se archiva hasta que estas dos comprobaciones queden anotadas acá:

1. **El selector en una consola real**, en Windows (Windows Terminal y la consola clásica) y en una terminal de Unix. Con la entrada y la salida en la terminal, `clawg preferences set review-model` abre el selector; `↑` y `↓` mueven la barra; Enter o Espacio abren una familia y una versión; Esc vuelve un paso; una pulsación marca un modelo y otra sobre el marcado lo guarda en `review.json`; `q` y Ctrl-C salen con 0 y dejan el archivo como estaba; al salir, la consola muestra lo que tenía y vuelve a hacer eco de lo que se escribe. Si las flechas no llegan como secuencias en Windows, se lee con `ReadConsoleInputW`, como dice Decisión.
2. **H19 en Linux**: que el CI compile `review/terminal.rs` y pase `terminal::tests::the_termios_for_the_picker` y los dos clippy.
