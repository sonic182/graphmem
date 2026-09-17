# Sugerencias de mejora

Ideas para subir los números de [evals_results.md](evals_results.md), sin
tocar el enfoque base (grafo de entidades + embeddings + PageRank), que ya
quedó validado en los tres datasets.

## Modelo de embeddings

- **Probar un modelo orientado a retrieval multi-hop o más grande.**
  `msmarco-distilbert-cos-v5` es genérico y, sin grafo, queda flojo o mixto
  frente a BM25 (ver los modos sin grafo en
  [evals_results.md](evals_results.md)). El propio Qwen3 que ya
  soporta `embedding.rs` es candidato, o algún modelo de la familia
  `bge`/`gte` entrenado para QA multi-salto.
- **Instrucción específica en `embed_query`.** Ya se usa un prefijo
  `Instruct:` para Qwen3; probar variantes de esa instrucción (o añadir una
  para DistilBERT, si el checkpoint la soporta) puede mover el recall sin
  grafo.

## Parámetros de `[retrieval]`

- **Barrer `damping` y `memory_seed_weight`** de Personalized PageRank en los
  tres datasets. Hoy están sin tocar; nunca se comparó lado a lado.
- **Semillas de entidad ponderadas por IDF**, dentro de gmem en vez del tope
  de frecuencia del script de evaluación (`--spacy-max-df`/`--spacy-min-df`).
  Hoy una entidad frecuente se descarta por completo; pesarla por IDF podría
  rescatar señal sin la sobrecarga de PageRank ruidoso que causó el fallback
  al tope de frecuencia (ver [conclusions.md](conclusions.md) y
  [evals_results.md](evals_results.md)).

## Extracción de relaciones (sin LLM)

- **spaCy extrae muy pocas tripletas** (menos de 1 por párrafo). La conexión
  entre párrafos viene sobre todo de entidades compartidas, no de relaciones.
  Probar reglas de extracción más ricas (coordinación, aposición, pronombres
  resueltos con coreferencia) o un modelo spaCy más grande
  (`en_core_web_trf`) para ver si sube la cantidad y calidad de tripletas.
- **La brecha con `oracle` mide el margen real:** en 2Wiki, recall@10 pasa de
  0.935 (spacy filtrado) a 0.985 (oracle); en MuSiQue, de 0.910 a 1.000.
  Cerrar esa brecha es el mayor potencial de mejora identificado hasta ahora.

## Evaluación

- **Más preguntas por dataset** (100 o más) para que cada punto de recall no
  dependa de 1-2 preguntas y las diferencias de ±0.03 sean fiables.
- **Medir el tiempo de CPU en MuSiQue** con y sin grafo — hoy solo hay
  números de WGPU, así que no se puede calcular la aceleración real ahí como
  en HotpotQA/2Wiki.
- **`oracle` en HotpotQA:** el dataset no trae tripletas propias, así que no
  hay techo ahí; si se quiere un techo comparable, habría que derivarlo de
  otra fuente (p. ej. los `supporting_facts`).

## Rendimiento

- **Compilar con `RUSTFLAGS="-C target-cpu=native"` (o `x86-64-v3`).** Hoy
  `gmem` se compila para x86-64 genérico (solo SSE2); los kernels SIMD
  propios de candle están condicionados a `target_feature = "avx2"` en
  tiempo de compilación, así que en CPU no se están usando aunque la CPU los
  soporte. La multiplicación de matrices (`gemm`) ya detecta AVX2/FMA en
  runtime, pero el resto de operaciones elemento a elemento no.
- **Reutilizar vectores entre grafos.** Hoy cada combinación
  (dataset, grafo) reembebe desde cero (~0.36 s por memoria); si el texto no
  cambia entre grafos, cachear el vector de la memoria y solo reembeber
  entidades/aristas nuevas ahorraría la mayor parte del tiempo de cada
  evaluación.
- **Medir el batch por defecto de WGPU con un corpus más grande.** Con 100
  párrafos, el batch no cambió el tiempo total (11.2–11.7 s de batch 1 a
  16): domina la carga del modelo. Con más texto por medición debería verse
  el efecto del batch y se podría fijar un valor por defecto con datos
  reales en vez del provisional (8).
- **Proponer `enqueue_64_big`** (ya usado en `binary.rs` del fork de candle)
  para las operaciones que aún usan `enqueue_64` (softmax, máscara, matmul),
  en el PR #3379 de candle. Quitaría la necesidad del batching por
  presupuesto de tokens en gmem para quien use el fork directamente.
