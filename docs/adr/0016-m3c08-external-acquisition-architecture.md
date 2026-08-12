# ADR 0016: M3C-08 external-acquisition architecture review

Status: Accepted for M3C-08 design.

M3C-08 is a design-only comparison of conceptual acquisition families for
the M3C-07 external-evidence contract. It selects no Windows API, dependency,
or implementation mechanism and performs no OS/editor inspection.

## Capability matrix

| Family | Text | Caret/selection | Logical identity | IME/composition | Secure evidence | Session evidence | Operation units | Freshness | Atomicity |
|---|---|---|---|---|---|---|---|---|---|
| UI Automation/accessibility providers | PARTIAL - provider text ranges may expose content, but normalization and custom-provider behavior vary | PARTIAL - ranges may describe a caret or selection, but multi-selection and custom surfaces are not uniform | PARTIAL - provider/control identity is stronger than a window, but can be unstable | UNKNOWN - generic provider evidence does not establish composition state | PARTIAL - protected controls may redact or expose metadata; denial must fail closed | WEAK - provider data does not by itself prove local/remote session | PARTIAL - provider-native ranges are not scalar or editor-operation proof | PARTIAL - queries are snapshots and can disagree | CHECK_THEN_ACT_RACE - no generic binding to later mutation is established |
| TSF/text services | PARTIAL - text-store access can be rich, but availability depends on the text service | PARTIAL - text-store ranges can represent selection/caret when the store cooperates | PARTIAL - text-store identity is promising, but lifecycle binding remains adapter-specific | STRONG - composition is a native concern, subject to service availability and lifecycle | WEAK - secure-field semantics are not guaranteed by the family alone | WEAK - session locality is not established by text services | PARTIAL - provider-native ranges may be useful, but mapping must be proven per store | PARTIAL - live state can change during separate reads | UNKNOWN - no general atomic validation-to-mutation primitive is assumed |
| Application-specific adapter | STRONG - cooperating host can return the exact logical snapshot | STRONG - host can define caret, selection, and multi-caret semantics | STRONG - editor/document identity can be explicit | PARTIAL - host must expose composition state correctly | STRONG - host can veto protected surfaces without reading secrets | PARTIAL - host can report session policy, but deployment still matters | STRONG - host can define its own operation units | STRONG - one host snapshot can be coherent | ATOMIC_PRIMITIVE_POSSIBLE - only if the host explicitly binds validation and replacement |
| Controlled synthetic/test surface | STRONG - deterministic owned text surface | STRONG - deterministic caret/selection controls | STRONG - owned surface identity is explicit | STRONG - composition state can be scripted | STRONG - secure state can be scripted without secret text | STRONG - session states can be modeled | STRONG - operation units are explicit test contracts | STRONG - snapshots and transitions are deterministic | ATOMIC_PRIMITIVE_POSSIBLE - test hooks can bind or inject races deliberately |

Ratings are architecture evidence, not production passes. PARTIAL or UNKNOWN
does not satisfy an M3C-07 requirement.

## Findings

UI Automation and accessibility providers are useful for cooperative standard
controls, but provider ranges, caret state, protected metadata, and custom
editor behavior are not uniform. Reading a range does not prove that the same
range remains at the caret or is still current.

TSF is the most natural conceptual source for composition and text-store
semantics, but availability and lifecycle vary by application and input
service. It does not automatically prove secure state, session locality, or
atomicity.

Application-specific adapters can provide the strongest identity, exact text,
operation-unit, and freshness evidence when a host cooperates. They require
explicit integration and cannot claim broad application coverage.

The controlled synthetic surface is not a production mechanism. It can make
exact matching, zero selection, logical identity, operation units, secure
vetoes, lifecycle invalidation, and race injection deterministic before any
OS-facing experiment.

## Cross-provider and external constraints

No single generic family is shown to satisfy every requirement. Composing a
text/caret provider with composition, secure, or session providers can improve
coverage, but independently queried providers may describe different target
states and make freshness and atomicity harder. Unknown or unavailable
evidence remains fail-closed.

Elevated targets, sandboxed or custom-rendered surfaces, unsupported providers,
cross-integrity restrictions, and protected fields are compatibility boundaries.
The architecture makes no universal-coverage claim:

- standard Win32 edit controls: SUPPORTED_CANDIDATE, subject to provider evidence;
- Chromium/Electron, VS Code-like, Office, and terminal surfaces: PARTIAL;
- custom-rendered editors and secure/password fields: UNSUPPORTED/UNKNOWN until
  explicit evidence exists;
- browser text fields: PARTIAL and provider-dependent.

## Exactness, freshness, and race

Every future adapter must bind the exact rendered snapshot to the actual caret,
reject non-empty selection initially, distinguish identical tokens in one
surface, and state its text unit. Scalar counts are insufficient. A sequence of
reads followed by later mutation is a CHECK_THEN_ACT_RACE. Only an explicit
atomic host primitive, or an immediate fail-closed recheck with documented
limits, can improve that classification; no such primitive is selected here.

## Recommendation

**C - controlled test surface should come before production OS acquisition.**

The unresolved atomicity and exact text/caret binding risks are more important
than API convenience. A deterministic owned surface can test the evidence
envelope, operation units, secure fail-closed behavior, lifecycle changes, and
race injection without selecting or invoking OS APIs. Owner accepted this
recommendation; the next milestone should design that test surface only.

No mutation technology, OS inspection, dependency, or Windows implementation
is selected by this ADR.
