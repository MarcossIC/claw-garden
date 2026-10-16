# embedder-idle-unload — Verificación

## Linux

Sobre el último commit de la rama, una sola corrida (regla del proyecto):

- `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- `cargo test --workspace`: **2.065 pasan, 0 fallan, 0 ignorados.** En `main` (`84ee27b`) pasaban 2.044; el cambio suma 21 tests: 14 en `idle_embedder.rs`, 1 en `bootstrap.rs`, 5 en `tests/api.rs` y 1 en `client.rs`.

**Paso 0** con el modelo real, cinco cargas y descargas en un proceso ([diseño, D1](design.md#d1-la-memoria-vuelve-medición)): tras soltar el modelo, 41-60 MB residentes en las cinco vueltas (659-762 MB sin el trim), y el mismo vector en todas.

`rag-eval` no aplica: los vectores son los mismos y no cambia el ranking.

## CI

- [ ] El workflow en verde en `ubuntu-latest` y `windows-latest` sobre el último commit del PR.

## Windows (máquina de Marcos)

- [ ] `cargo test --workspace --target-dir <otro-dir>`: la base más 21 tests.
- [ ] `cargo clippy --workspace --all-targets -- -A clippy::all -D warnings`: código 0.
- [ ] **La memoria vuelve en Windows.** Con el daemon dev de este build y `CLAWG_EMBEDDER_IDLE_MINUTES=1` en su entorno:
  1. Una búsqueda (`clawg memories search prueba --profile dev`) y anotar los bytes privados: `(Get-Process -Id (Get-Content ~/.clawg-dev/daemon.json | ConvertFrom-Json).pid).PrivateMemorySize64`.
  2. Esperar dos minutos sin usarlo. El log dice `embedding model unloaded after 60 s without use`, `clawg status --profile dev` muestra `"embedder_resident": false` y los bytes privados bajan a menos de 300 MB.
  3. Otra búsqueda: responde, el log dice `embedding model files read and checked in … ms, session built in … ms` y `embedding model loaded again in … ms`, y `embedder_resident` vuelve a `true`.
  4. Repetir 2 y 3 tres veces: los bytes privados tras soltarlo no crecen de una vuelta a otra.
- [ ] Anotar los tiempos de la recarga del log (lectura con hash y sesión): dicen si saltear el SHA-256 en las recargas vale la pena.
