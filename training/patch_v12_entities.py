import sys

with open('/workspace/ApexIntel/training/eval_harness.py', 'r') as f:
    content = f.read()

# Patch 1: Add actors handling before flat entity handler
old = '    # Handle flat entity format (single entity at root, e.g., adversarial tests)\n    if "name" in obj and "companies" not in obj and "persons" not in obj:'

new = '''    # Handle actors array (maps to persons/companies)
    for entry in obj.get("actors", []) or []:
        if isinstance(entry, dict):
            role = (entry.get("role", "") or "").lower()
            name = entry.get("name", "")
            if "person" in role:
                add("person", name)
            else:
                add("company", name)
        else:
            add("company", str(entry))

    # Handle flat entity format (single entity at root, e.g., adversarial tests)
    if ("name" in obj or "entity_name" in obj) and "companies" not in obj and "persons" not in obj:'''

if old in content:
    content = content.replace(old, new)
    print('PATCH 1 OK: actors + entity_name guard')
else:
    print('PATCH 1 FAIL: pattern not found')
    sys.exit(1)

# Patch 2: Use entity_name field in flat handler
old2 = '        etype = (obj.get("type", "") or obj.get("entity_type", "") or "company").lower()\n        if etype in ("company", "ems", "oem", "manufacturer"):\n            add("company", obj.get("name", ""))\n        elif etype == "person":\n            add("person", obj.get("name", ""))\n        else:\n            add("company", obj.get("name", ""))'

new2 = '        ename = obj.get("name", "") or obj.get("entity_name", "")\n        etype = (obj.get("type", "") or obj.get("entity_type", "") or "company").lower()\n        if etype in ("company", "ems", "oem", "manufacturer"):\n            add("company", ename)\n        elif etype == "person":\n            add("person", ename)\n        else:\n            add("company", ename)'

if old2 in content:
    content = content.replace(old2, new2)
    print('PATCH 2 OK: entity_name field')
else:
    print('PATCH 2 FAIL: pattern not found')
    sys.exit(1)

# Patch 3: Extract entities from summary/full_text as fallback
# Find the return statement inside collect_entities
idx = content.find('def collect_entities')
if idx >= 0:
    ret_idx = content.find('    return collected\n', idx)
    if ret_idx >= 0:
        replacement = '''    # Extract from summary/full_text for adversarial tests
    for text_field in ("summary", "full_text"):
        txt = obj.get(text_field, "") or ""
        if txt and not collected:
            import re as _re
            for match in _re.findall(r'[A-Z][a-z]+(?:\\s+[A-Z][a-z]+)+', txt):
                if len(match) > 3:
                    add("company", match)

    return collected
'''
        content = content[:ret_idx] + replacement + content[ret_idx + len('    return collected\n'):]
        print('PATCH 3 OK: summary/full_text extraction')
    else:
        print('PATCH 3 SKIP: return not found')
else:
    print('PATCH 3 SKIP: collect_entities not found')

with open('/workspace/ApexIntel/training/eval_harness.py', 'w') as f:
    f.write(content)

print('All patches applied successfully')
