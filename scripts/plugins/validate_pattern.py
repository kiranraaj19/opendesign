"""Example OpenDesign pattern-validation plugin.

Loaded by `od_core::plugin::run_validation_script` (behind the
`python-plugins` cargo feature). A validation script just needs a
top-level `validate` function with this signature:

    def validate(name: str, point_count: int, seam_allowance_mm: float) -> str:
        ...

Return an empty string if the pattern looks fine, or a human-readable
message describing the problem otherwise. OpenDesign doesn't interpret
the message beyond displaying it, so a plugin can report whatever rules
its studio cares about (minimum point counts, seam allowance ranges,
naming conventions, ...).

This example is intentionally simple: it flags patterns with fewer than
4 outline points (likely too simple to be a real garment piece) and
patterns with no seam allowance at all.
"""


def validate(name: str, point_count: int, seam_allowance_mm: float) -> str:
    problems = []
    if point_count < 4:
        problems.append(
            f"only {point_count} outline point(s); expected at least 4"
        )
    if seam_allowance_mm <= 0.0:
        problems.append("no seam allowance set")
    if not problems:
        return ""
    return f"'{name}': " + "; ".join(problems)
