#!/bin/bash
# =============================================================================
# Neuron NODE Bootstrap — MacBook Pro
# Run this ON the MacBook to extract data and set up remote access for Ambrose
# =============================================================================

set -e

EXPORT_DIR="$HOME/neuron_export"
AMBROSE_IP="192.168.0.233"
AMBROSE_USER="Nick"

echo "=== Neuron NODE Bootstrap ==="
echo "Export dir: $EXPORT_DIR"
mkdir -p "$EXPORT_DIR"

# ── 1. Enable Remote Login (SSH) if not already ──────────
echo ""
echo "[1/6] Checking SSH..."
if sudo systemsetup -getremotelogin 2>/dev/null | grep -q "Off"; then
    echo "  Enabling Remote Login (SSH)..."
    sudo systemsetup -setremotelogin on
    echo "  SSH enabled."
else
    echo "  SSH already enabled."
fi

# ── 2. Extract iMessage database ──────────────────────────
echo ""
echo "[2/6] Extracting iMessage database..."
MSG_DB="$HOME/Library/Messages/chat.db"
if [ -f "$MSG_DB" ]; then
    # Need Full Disk Access for Terminal to read this
    cp "$MSG_DB" "$EXPORT_DIR/chat.db" 2>/dev/null && echo "  Copied chat.db ($(du -sh "$EXPORT_DIR/chat.db" | cut -f1))" || {
        echo "  ERROR: Can't access chat.db — grant Terminal Full Disk Access:"
        echo "  System Settings → Privacy & Security → Full Disk Access → Terminal (toggle on)"
        echo "  Then re-run this script."
    }
    # Also grab the attachments database
    [ -f "$HOME/Library/Messages/chat.db-wal" ] && cp "$HOME/Library/Messages/chat.db-wal" "$EXPORT_DIR/"
    [ -f "$HOME/Library/Messages/chat.db-shm" ] && cp "$HOME/Library/Messages/chat.db-shm" "$EXPORT_DIR/"
else
    echo "  chat.db not found (Messages not on this Mac?)"
fi

# ── 3. Extract Contacts ──────────────────────────────────
echo ""
echo "[3/6] Extracting Contacts..."
CONTACTS_DIR="$HOME/Library/Application Support/AddressBook"
if [ -d "$CONTACTS_DIR" ]; then
    mkdir -p "$EXPORT_DIR/contacts"
    find "$CONTACTS_DIR" -name "*.abcddb" -exec cp {} "$EXPORT_DIR/contacts/" \; 2>/dev/null
    find "$CONTACTS_DIR" -name "*.sqlitedb" -exec cp {} "$EXPORT_DIR/contacts/" \; 2>/dev/null
    echo "  Contacts exported: $(ls "$EXPORT_DIR/contacts/" 2>/dev/null | wc -l) files"
else
    echo "  AddressBook directory not found"
fi

# ── 4. Extract Notes ─────────────────────────────────────
echo ""
echo "[4/6] Extracting Notes..."
NOTES_DIR="$HOME/Library/Group Containers/group.com.apple.notes"
if [ -d "$NOTES_DIR" ]; then
    mkdir -p "$EXPORT_DIR/notes"
    find "$NOTES_DIR" -name "*.sqlite" -exec cp {} "$EXPORT_DIR/notes/" \; 2>/dev/null
    echo "  Notes exported: $(ls "$EXPORT_DIR/notes/" 2>/dev/null | wc -l) files"
else
    echo "  Notes directory not found"
fi

# ── 5. Extract Safari history ────────────────────────────
echo ""
echo "[5/6] Extracting Safari history..."
SAFARI_DB="$HOME/Library/Safari/History.db"
if [ -f "$SAFARI_DB" ]; then
    cp "$SAFARI_DB" "$EXPORT_DIR/safari_history.db" 2>/dev/null && \
        echo "  Safari history: $(du -sh "$EXPORT_DIR/safari_history.db" | cut -f1)" || \
        echo "  ERROR: Can't access Safari history — needs Full Disk Access"
else
    echo "  Safari history not found"
fi

# ── 6. Extract Calendar ──────────────────────────────────
echo ""
echo "[6/6] Extracting Calendar..."
CAL_DIR="$HOME/Library/Calendars"
if [ -d "$CAL_DIR" ]; then
    mkdir -p "$EXPORT_DIR/calendar"
    find "$CAL_DIR" -name "*.sqlite" -exec cp {} "$EXPORT_DIR/calendar/" \; 2>/dev/null
    find "$CAL_DIR" -name "*.ics" -exec cp {} "$EXPORT_DIR/calendar/" \; 2>/dev/null
    echo "  Calendar exported: $(ls "$EXPORT_DIR/calendar/" 2>/dev/null | wc -l) files"
else
    echo "  Calendar directory not found"
fi

# ── Summary ──────────────────────────────────────────────
echo ""
echo "=== Export Complete ==="
du -sh "$EXPORT_DIR"
echo ""
echo "Files exported to: $EXPORT_DIR"
ls -la "$EXPORT_DIR"
echo ""

# ── Copy to Ambrose ──────────────────────────────────────
echo "Copying to Ambrose ($AMBROSE_IP)..."
echo "Run this command to transfer:"
echo "  scp -r $EXPORT_DIR $AMBROSE_USER@$AMBROSE_IP:D:/EVA/SUBSTRATE/data/raw/macbook/"
echo ""
echo "Or if you want to do it now, press Enter (or Ctrl+C to skip):"
read -r
scp -r "$EXPORT_DIR" "$AMBROSE_USER@$AMBROSE_IP:D:/EVA/SUBSTRATE/data/raw/macbook/" && \
    echo "Transfer complete!" || \
    echo "Transfer failed — you may need to enable SSH on Ambrose or use a USB drive."
