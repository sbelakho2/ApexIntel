# NER Models Training Data

## Recommended Pre-trained Models for Fine-tuning

### For Entity Extraction (Company, Person, Location, Product)
- **dslim/bert-base-NER** - HuggingFace: BERT fine-tuned on CoNLL-2003
  Download: `huggingface-cli download dslim/bert-base-NER`
- **Jean-Baptiste/camembert-ner** - French NER model
  Download: `huggingface-cli download Jean-Baptiste/camembert-ner`
- **Davlan/xlm-roberta-large-ner-hrl** - Multilingual NER (10 languages)
  Download: `huggingface-cli download Davlan/xlm-roberta-large-ner-hrl`

### For Relation Extraction
- **Babelscape/rebel-large** - Relation extraction from text
  Download: `huggingface-cli download Babelscape/rebel-large`

### For Entity Resolution / Deduplication
- **sentence-transformers/all-MiniLM-L6-v2** - Embedding model for fuzzy matching
  Download: `huggingface-cli download sentence-transformers/all-MiniLM-L6-v2`

### For Domain-Adaptive Pre-Training (DAPT)
- **meta-llama/Llama-3.1-8B** - Base model for OSINT domain adaptation
- **Qwen/Qwen3-30B-A3B** - Primary production model (matches llm crate config)
- **mistralai/Mistral-Nemo-Instruct-2407** - Fast inference model for worker nodes

### Training Datasets on HuggingFace
- **conll2003** - Standard NER benchmark (English)
- **wnut_17** - Emerging/novel entity recognition
- **tner/ontonotes5** - OntoNotes 5.0 (18 entity types)
- **numind/NuNER** - Universal NER (diverse domains)
- **mit-ll/SPEED** - Sanctioned party entity detection
