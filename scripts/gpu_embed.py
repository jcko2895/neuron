#!/usr/bin/env python3
"""
GPU-accelerated embedding for Neuron -> MemPalace.
Uses sentence-transformers on RTX 3090 for ~5000+ embeddings/sec.
"""

import json
import sys
import time
import os
from pathlib import Path

import torch
from sentence_transformers import SentenceTransformer

INPUT = sys.argv[1] if len(sys.argv) > 1 else "D:/EVA/SUBSTRATE/data/neuron_full_export.jsonl"
OUTPUT = sys.argv[2] if len(sys.argv) > 2 else "D:/EVA/SUBSTRATE/data/mempalace_drawers.jsonl"
MODEL_NAME = "all-MiniLM-L6-v2"  # 384-dim, fast, good quality
BATCH_SIZE = 512  # RTX 3090 can handle large batches

def platform_to_wing(platform):
    mapping = {
        "facebook": "social", "instagram": "social", "imessage": "social", "snapchat": "social",
        "gmail": "email",
        "spotify": "music", "apple_music": "music",
        "youtube": "activity", "google_activity": "activity",
        "google_chrome": "browsing", "edge": "browsing", "safari": "browsing", "firefox": "browsing",
        "chatgpt": "ai_conversations", "claude": "ai_conversations", "codex": "ai_conversations",
        "google_calendar": "calendar",
        "google_contacts": "people", "apple_contacts": "people",
        "apple_photos": "photos",
        "apple_notes": "notes",
    }
    return mapping.get(platform, "general")

def main():
    print("=== Neuron -> MemPalace GPU Loader ===")
    print(f"Device: {torch.cuda.get_device_name(0)}")
    print(f"Model:  {MODEL_NAME}")
    print(f"Batch:  {BATCH_SIZE}")
    print(f"Input:  {INPUT}")
    print(f"Output: {OUTPUT}")

    # Resume support
    already_done = 0
    if os.path.exists(OUTPUT):
        with open(OUTPUT, 'r', encoding='utf-8') as f:
            already_done = sum(1 for _ in f)
        print(f"Resuming from record {already_done}")

    # Load model onto GPU
    print("Loading model...")
    model = SentenceTransformer(MODEL_NAME, device='cuda')
    print(f"Model loaded. Embedding dim: {model.get_sentence_embedding_dimension()}")

    # Read all records
    print("Reading JSONL...")
    records = []
    with open(INPUT, 'r', encoding='utf-8') as f:
        for i, line in enumerate(f):
            if i < already_done:
                continue
            line = line.strip()
            if not line:
                continue
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError:
                continue

    total = len(records)
    print(f"Records to embed: {total:,}")

    # Process in batches
    start = time.time()
    embedded = 0

    out_file = open(OUTPUT, 'a', encoding='utf-8')

    for batch_start in range(0, total, BATCH_SIZE):
        batch = records[batch_start:batch_start + BATCH_SIZE]

        # Truncate long content for embedding
        texts = []
        for r in batch:
            text = r.get('content', '')
            if len(text) > 1000:
                text = text[:1000]
            texts.append(text)

        # Embed on GPU
        embeddings = model.encode(texts, batch_size=BATCH_SIZE, show_progress_bar=False,
                                   convert_to_numpy=True, normalize_embeddings=True)

        # Write drawers
        for record, embedding in zip(batch, embeddings):
            drawer = {
                "id": record.get("content_hash", ""),
                "content": record.get("content", ""),
                "wing": platform_to_wing(record.get("platform", "")),
                "room": record.get("thread_name", "") or record.get("source_type", ""),
                "source": record.get("source_file", ""),
                "metadata": {
                    "timestamp": record.get("timestamp"),
                    "actor": record.get("actor"),
                    "is_user": record.get("is_user"),
                    "platform": record.get("platform"),
                    "source_type": record.get("source_type"),
                    "thread_id": record.get("thread_id"),
                },
                "embedding": embedding.tolist(),
            }
            out_file.write(json.dumps(drawer, ensure_ascii=False) + '\n')

        embedded += len(batch)

        # Progress
        elapsed = time.time() - start
        rate = embedded / elapsed if elapsed > 0 else 0
        remaining = (total - embedded) / rate if rate > 0 else 0

        if embedded % (BATCH_SIZE * 10) == 0 or embedded == total:
            print(f"  [{elapsed:.0f}s] {embedded:,}/{total:,} ({embedded*100//total}%) — {rate:.0f}/sec — ~{remaining/60:.1f}min left")

    out_file.close()

    elapsed = time.time() - start
    print(f"\n=== Done ===")
    print(f"  Embedded: {embedded:,}")
    print(f"  Time:     {elapsed:.1f}s ({elapsed/60:.1f} min)")
    print(f"  Rate:     {embedded/elapsed:.0f}/sec")
    print(f"  Output:   {OUTPUT}")

if __name__ == "__main__":
    main()
