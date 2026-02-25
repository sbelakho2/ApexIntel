# Sensei-Rams Anti-Patterns & Component Catalog (ApexIntel)

> Quick reference to prevent drift into generic SaaS styling.

---

## 1. Buttons

### ❌ Avoid

- Pill buttons (`rounded-full`)
- Gradient fills
- Marketing copy (`Get Started`, `Amazing!`)

### ✅ Use

- Micro radius (`rounded-rams-sm`)
- Border + solid fill
- Operational verbs (`Run`, `Acknowledge`, `Promote`, `Export`)

---

## 2. Cards / Containers

### ❌ Avoid

- Floating cards with large outer shadows
- Excessive padding and decorative empty whitespace

### ✅ Use

- Module containers (`bg-rams-module border border-rams-line`)
- Header divider + compact content body
- Dense but readable vertical rhythm

---

## 3. Navigation

### ❌ Avoid

- Avatar-centric personal sidebar chrome
- Bright color blocks and playful highlights

### ✅ Use

- Rack metaphor
- Active indicator rail/dot
- Neutral module surfaces + semantic accent only for state

---

## 4. Status Signals

### ❌ Avoid

- Color-only badges
- Pulsing decorative animations as sole signal

### ✅ Use

- Icon + label + color
- Monospace/uppercase metadata for operational states

---

## 5. Data Tables

### ❌ Avoid

- Soft striped visual noise as primary hierarchy
- oversized row padding

### ✅ Use

- clear border structure
- compact uppercase headers
- tabular numeric rendering for values

---

## 6. Forms

### ❌ Avoid

- floating animated labels
- overly rounded controls

### ✅ Use

- fixed labels above inputs
- compact inset panel styling
- clear focus treatment

---

## 7. Copy Tone

### ❌ Avoid

- consumer/app-store language
- exclamation-heavy messaging

### ✅ Use

- operator-focused, concrete language
- unambiguous status and action text

---

## 8. Enforced UX Heuristics

Every new component should pass:

- Is this element functional, not decorative?
- Is hierarchy clear without motion?
- Does this fit the control-station metaphor?
- Can we remove one layer and improve clarity?

