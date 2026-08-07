# M2 English/IT detection design

M2 is a deterministic decision layer, not an input runtime. Its inputs are the
raw ASCII token, the Telex-rendered token, a typed boundary, and an explicit
InputContext. Its outputs are a typed RecoveryDecision and, only for approved
recovery, an optional grapheme-safe EditPlan.

## Classification order

1. Reject empty, non-ASCII raw input and unknown boundaries as unsupported.
2. Protect structured technical syntax: URLs and URIs, email, Windows/Unix/UNC
   paths, domains, IP addresses, command options, identifiers, semantic
versions, UUIDs, and hex/hash-like values.
   Secret-like long mixed-class values and common access-key prefixes are also
   blocked conservatively.
3. If raw and rendered differ, perform a case-insensitive exact lookup in the
   product, technical, English, and injected user dictionaries.
4. Treat an unmatched rendered token containing precomposed Vietnamese letters
   or tone marks as a Vietnamese candidate.
5. Return unknown evidence otherwise.

There is no fuzzy lookup, language model, frequency model, locale dependency,
clock, randomness, network access, or mutable global state.

## Confidence and policy

Confidence is a validated finite value in the inclusive range zero to one.
Exact English matches carry 0.98; exact technical and product matches carry
0.99; exact injected user allow-list matches carry 1.00.

- Writing permits exact evidence at 0.98 or above.
- Technical editors require 0.99 and technical, product, or user evidence.
- Terminal, secure, remote, and unknown contexts always veto recovery.
- Never-transform and Vietnamese evidence always keep the rendered token.
- Unknown or insufficient evidence is ambiguous and produces no edit.

Context comes from the caller. M2 makes no attempt to discover platform state.
The default is safe because uncertain evidence and uncertain context cannot
produce an automatic recovery plan.

M2 is observe-only with respect to keyboard input: it neither receives global
keyboard events nor executes its edit plans.

## Known tradeoffs

Exact lookup intentionally creates false negatives for legitimate English and
technical words outside the curated lists. Conservative structure recognition
can also block prose containing identifier-like casing. These misses are safer
than restoring Vietnamese or rewriting code, secrets, and operational commands.
The Vietnamese check is deliberately one-sided: a precomposed Vietnamese mark
blocks an unknown token, but M2 does not claim full Vietnamese word validation.

## Boundary and casing behavior

Dictionary matching ignores ASCII casing, but restoration uses the exact raw
text supplied by the caller. The boundary is not part of dictionary lookup, so
resume followed by a period can be recovered while a complete token such as
server.local is protected as a domain. Punctuation is not included in the edit.
