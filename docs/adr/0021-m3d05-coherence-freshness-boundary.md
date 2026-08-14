# ADR 0021: M3D-05 coherence and freshness boundary

Status: Accepted

The standard-`EDIT` diagnostic probe performs two bounded samples. Each sample
reads control identity/style, text, and selection. The probe accepts only when
the samples are equal and the second sample has an empty selection whose exact
UTF-16 prefix matches the expected token. Any read failure, timeout,
identity/style change, text change, selection change, or contradiction rejects
fail-closed.

This is `PARTIAL_STALENESS_DETECTION_ONLY`. It detects changes observed between
the two read windows with a fixed two-sample bound. It does not create an
atomic snapshot, prove freshness at any later time, establish cross-query
coherence, or close a validation-to-mutation race. It does not address IME,
session/security completeness, or editor-native operation units.

No retry loop is unbounded, and no mutation or synchronization hook is added.
