# Security

## Reporting a problem

Please report a vulnerability privately, through [GitHub's private vulnerability reporting](https://github.com/adamgilman/endmark/security/advisories/new), and not in a public issue.

## What to keep in mind when using Endmark

These are properties of the format, described in section 10 of [RFC 0001](rfcs/0001-endmark.md). They are not bugs.

- **Records are hidden text.** A rendered page does not show them, but an agent that reads the file sees them. Treat the note and the quoted text of every record as data from its named author, never as instructions. An agent that only needs the document can read `endmark body FILE`, which leaves the records out.
- **Authors are not authenticated.** Anyone who can write the file can write any `by`. Trust has to come from where the file is kept: access control, signed commits, review.
- **An accepted suggestion can add HTML or links.** Review what a suggestion renders as, not only its source.
