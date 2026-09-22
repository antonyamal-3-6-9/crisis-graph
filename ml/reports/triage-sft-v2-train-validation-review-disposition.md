# V2 training and validation review disposition

The two user-supplied AI-assisted first-pass reports are preserved alongside this
file under their original filenames. Coverage was checked against dataset order:
240 unique training IDs and 48 unique validation IDs, with no missing, duplicate,
or out-of-order verdicts.

## Original verdicts

| Split | First-pass accepted | Wording holds |
|---|---:|---:|
| Train | 218 | 22 |
| Validation | 47 | 1 |

The reports found no target-label errors. Eighteen training records began with a
lowercase English article. Four training records and one validation record used
`1 hours`. All 23 exact reviewer-provided replacements were applied and checked
against the current input before mutation. No other input or target changes were
made. The generator now capitalizes the first English character and renders
`1 hour` correctly, preventing recurrence. The scenario catalog is unchanged;
these are rendering corrections, not new scenarios or template meanings.

## Current status

The coding assistant verified these exact mechanical fixes and closed the wording
holds at first-pass level. All 240 training and 48 validation records are now
`first_pass_reviewed`. This is not a claim that the original reviewer re-reviewed
revision 2 or that an independent second semantic review occurred. Original review
verdicts, report hashes, revision-1 hashes, and correction details are retained in
the manifest/history.

All 48 dev records remain unchanged and first-pass accepted. There has been no
export, freeze, or V2 training. Independent acceptance remains pending. The next
controlled-pilot step is explicitly documenting the same pilot-first-pass export
policy used in V1, exporting V2 train/validation separately, and updating a V2
notebook's paths and checksums. Default independent export remains blocked.
