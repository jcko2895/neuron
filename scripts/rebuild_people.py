#!/usr/bin/env python3
"""Rebuild people.json from neuron_full_export.jsonl with proper scoring."""

import json, sys, sqlite3, os, unicodedata
from collections import defaultdict

sys.stdout.reconfigure(encoding='utf-8')

EXPORT = "D:/EVA/SUBSTRATE/data/neuron_full_export.jsonl"
OUTPUT = "D:/eva-app/src/data/people.json"
CONTACTS_DIR = "D:/EVA/SUBSTRATE/data/raw/macbook/contacts/AddressBook"

# Phone map
phone_map = {}
for root, dirs, files in os.walk(CONTACTS_DIR):
    for f in files:
        if not f.endswith('.abcddb'): continue
        try:
            conn = sqlite3.connect(os.path.join(root, f))
            c = conn.cursor()
            c.execute('SELECT r.ZFIRSTNAME, r.ZLASTNAME, p.ZFULLNUMBER FROM ZABCDRECORD r JOIN ZABCDPHONENUMBER p ON p.ZOWNER = r.Z_PK WHERE r.ZFIRSTNAME IS NOT NULL AND p.ZFULLNUMBER IS NOT NULL')
            for first, last, phone in c.fetchall():
                name = f'{first or ""} {last or ""}'.strip()
                if not name: continue
                digits = ''.join(c for c in phone if c.isdigit())
                if len(digits) >= 10:
                    phone_map['+1' + digits[-10:]] = name
                    phone_map[digits[-10:]] = name
            conn.close()
        except: pass

phone_map.update({'+12063534877': 'Jenny Lieu', '+16237608231': 'Zinnia Pualani', '+13232538827': 'Keith Baker'})

MSG_PLATFORMS = ('facebook', 'instagram', 'imessage', 'snapchat')

# Pass 1: thread analysis
threads = defaultdict(lambda: {'user_msgs': 0, 'members': set()})
thread_total = defaultdict(int)
thread_person = {}

print("Pass 1: analyzing threads...")
with open(EXPORT, 'r', encoding='utf-8') as f:
    for line in f:
        try: r = json.loads(line)
        except: continue
        platform = r.get('platform', '')
        if platform not in MSG_PLATFORMS: continue
        thread = r.get('thread_id') or r.get('thread_name') or 'none'
        key = f'{platform}|{thread}'
        actor = r.get('actor') or ''

        # Resolve phone
        resolved = actor
        if actor.startswith('+') or (actor and actor[0].isdigit() and len(actor) > 8):
            digits = ''.join(c for c in actor if c.isdigit())
            if len(digits) >= 10:
                resolved = phone_map.get('+1' + digits[-10:], phone_map.get(digits[-10:], actor))

        thread_total[thread] += 1
        if r.get('is_user'):
            threads[key]['user_msgs'] += 1
        else:
            threads[key]['members'].add(resolved)
            if thread not in thread_person:
                thread_person[thread] = set()
            thread_person[thread].add(resolved)

# Find 1:1 threads
one_on_one = {}
for thread, members in thread_person.items():
    if len(members) == 1:
        one_on_one[thread] = list(members)[0]

# Pass 2: score people by 1:1 thread totals
print("Pass 2: scoring people...")
person_total = defaultdict(int)
person_platforms = defaultdict(set)
person_first = {}
person_last = {}

for thread, person in one_on_one.items():
    person_total[person] += thread_total[thread]

with open(EXPORT, 'r', encoding='utf-8') as f:
    for line in f:
        try: r = json.loads(line)
        except: continue
        actor = r.get('actor') or ''
        if not actor or r.get('is_user'): continue
        platform = r.get('platform', '')
        ts = r.get('timestamp', '')

        resolved = actor
        if actor.startswith('+') or (actor and actor[0].isdigit() and len(actor) > 8):
            digits = ''.join(c for c in actor if c.isdigit())
            if len(digits) >= 10:
                resolved = phone_map.get('+1' + digits[-10:], phone_map.get(digits[-10:], actor))

        person_platforms[resolved].add(platform)
        if ts:
            if resolved not in person_first or ts < person_first[resolved]: person_first[resolved] = ts
            if resolved not in person_last or ts > person_last[resolved]: person_last[resolved] = ts

# Filter
business_words = ['amazon', 'facebook', 'google', 'twitter', 'reddit', 'netflix', 'newegg',
    'paypal', 'youtube', 'spotify', 'promo', 'newsletter', '.com', 'community', 'neighbors',
    'scheduling', 'zumiez', 'domino', 'hot topic', 'quora', 'thrillist', 'jackthreads',
    'chatgpt', 'capital one', 'solidworks', 'pizza hut', 'noreply', 'teamsnapchat']

def is_real(name):
    if name == 'Nicholas Wilson Towne': return False
    lower = name.lower()
    for b in business_words:
        if b in lower: return False
    if name.startswith('+'): return False
    latin = sum(1 for c in name if c.isascii() and c.isalpha())
    if latin < 3: return False
    non_ascii = sum(1 for c in name if ord(c) > 127)
    if non_ascii > len(name) * 0.3: return False
    if ' ' not in name and name == name.lower() and len(name) > 5: return False
    for c in name:
        if ord(c) > 127 and unicodedata.category(c).startswith('So'): return False
    plats = person_platforms.get(name, set())
    if plats <= {'gmail', 'google_contacts'}: return False
    if '@' in name or '=?' in name: return False
    return True

# Build output
# NATALIE CUNNINGHAM = HOME PROGRAM FRIEND. NOT AN EX.
# NATALIE BURBRIDGE = HIGH SCHOOL EX.
romantic = {'Jenny Lieu', 'Zinnia Pualani', 'Grace Beck', 'Natalie Burbridge', 'Vanessa Spencer', 'Maria Martinez', 'Lana Waltosz'}
romantic_info = {
    'Jenny Lieu': {'order': 7, 'period': '2017-present', 'note': 'Girlfriend, 9 years', 'first_text': '2017-04-01'},
    'Zinnia Pualani': {'order': 6, 'period': '2014-2015', 'note': 'Ex'},
    'Grace Beck': {'order': 3, 'period': '~2013-2014', 'note': 'High school ex'},
    'Natalie Burbridge': {'order': 2, 'period': '~2012-2013', 'note': 'High school ex'},
    'Vanessa Spencer': {'order': 4, 'period': '~2014', 'note': 'High school ex'},
    'Maria Martinez': {'order': 5, 'period': '~2014-2015', 'note': 'Ex'},
    'Lana Waltosz': {'order': 1, 'period': '2011-2012', 'note': 'First girlfriend, age 15-16'},
}

female_names = {'jenny','zinnia','grace','nina','caroline','lucy','sydney','kelli','natalie','audrey','jennine','alyssa','kathryn','elisabeth','elysabeth','madison','chelsea','teresa','jacqueline','nicole','shelby','vanessa','maria','danielle','meagan','sabrina','mikayla','kayla','tara','emma','courtney','julie','lana','savannah','aimie','carolina','mariah','kapprielle','mona','kim','ashley','kelly','sarah','hannah'}
male_names = {'keith','justin','grady','hayden','eric','jeff','jeffery','walter','tyler','gabe','david','fernando','randy','juan','chris','uriah','dominic','logan','eian','rutger','michael','mark','iain','truman','daniel','steven','kevin','joseph','andy','johnson','tobias','sean','carlos','glen','benjamin','anthony','bryce','oliver','braeden','marc','ernest','makun','troy','jake','ben','carl','levi','dennis','billy','kirk','alex','derek','caleb','john','jim'}

output = []
for name, total in person_total.items():
    if total < 3: continue
    if not is_real(name): continue
    plats = sorted(person_platforms.get(name, set()))
    if not plats: continue

    first = name.split()[0].lower()
    gender = 'F' if first in female_names else ('M' if first in male_names else '?')

    output.append({
        'name': name,
        'interactions': total,
        'platforms': plats,
        'first_seen': person_first.get(name),
        'last_seen': person_last.get(name),
        'aliases': [],
        'gender': gender,
        'relationship': 'romantic' if name in romantic else 'platonic',
        'romantic_info': romantic_info.get(name),
    })

# Name corrections (display names from contacts/platforms that are wrong)
name_fixes = {
    'Caroline McDaddy': 'Caroline McDaniel',
    'Sydney (Sister) Towne': 'Sydney Towne',
    'Jeff (Dad) Towne': 'Jeff Towne',
}
alias_additions = {
    'Caroline McDaniel': ['Caroline McDaddy', 'carolinecamille'],
    'Sydney Towne': ['Sydney (Sister) Towne'],
    'Jeff Towne': ['Jeff (Dad) Towne'],
}

for p in output:
    if p['name'] in name_fixes:
        old = p['name']
        p['name'] = name_fixes[old]
        p['aliases'] = sorted(set(p.get('aliases', []) + [old]))
    if p['name'] in alias_additions:
        p['aliases'] = sorted(set(p.get('aliases', []) + alias_additions[p['name']]))

output.sort(key=lambda x: -x['interactions'])

with open(OUTPUT, 'w', encoding='utf-8') as f:
    json.dump(output, f, indent=2, ensure_ascii=False)

print(f'Total people: {len(output)}')
print('Top 20:')
for p in output[:20]:
    r = ' *' if p['relationship'] == 'romantic' else ''
    print(f'  {p["interactions"]:>7,}  {p["name"]:30s}  {p["gender"]}  {p["platforms"]}{r}')
