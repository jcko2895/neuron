#!/bin/bash
# =============================================================================
# Neuron Full Pipeline: Ingest -> Embed -> MemPalace
# Run after new data exports are downloaded to raw/
# =============================================================================

set -e

NEURON_DIR="D:/neuron"
DATA_DIR="D:/EVA/SUBSTRATE/data"
EXPORT="$DATA_DIR/neuron_full_export.jsonl"
DRAWERS="$DATA_DIR/mempalace_drawers.jsonl"

echo "=== Neuron Full Pipeline ==="
echo ""

# Step 1: Ingest all sources to JSONL
echo "[1/3] Running Neuron ingest..."
cd "$NEURON_DIR"
RUST_LOG=neuron=info ./target/release/neuron-ingest-all.exe 2>&1 | grep -E "records|Total|Exported|SKIP|ERROR"
echo ""

# Step 2: GPU embed new records (has resume support)
echo "[2/3] GPU embedding (resume-aware)..."
python scripts/gpu_embed.py "$EXPORT" "$DRAWERS"
echo ""

# Step 3: Report
echo "[3/3] Pipeline complete."
RECORDS=$(wc -l < "$EXPORT")
DRAWERS_COUNT=$(wc -l < "$DRAWERS")
echo "  Records: $RECORDS"
echo "  Drawers: $DRAWERS_COUNT"
echo "  Export:  $EXPORT"
echo "  Drawers: $DRAWERS"
