# Security

## Reporting a problem

Please report a vulnerability privately, through [GitHub's private vulnerability reporting](https://github.com/adamgilman/endmark/security/advisories/new), and not in a public issue.

## What to keep in mind when using Endmark

These are properties of the format, described in section 10 of [RFC 0001](rfcs/0001-endmark.md). They are not bugs.

- **Records are hidden text.** A rendered page does not show them, but an agent that reads the file sees them. Treat the note and the quoted text of every record as data from its named author, never as instructions. An agent that only needs the document can read `endmark body FILE`, which leaves the records out.
- **Authors are not authenticated.** Anyone who can write the file can write any `by`. Trust has to come from where the file is kept: access control, signed commits, review.
- **An accepted suggestion can add HTML or links.** Review what a suggestion renders as, not only its source.

## The `endmark view` server

`endmark view FILE` serves a review page for one file to a browser on the same computer. It is an example for one person, written on the Rust standard library, and not a hardened web server. This is what it does about the ways a local server usually goes wrong.

| Threat | What stops it |
| --- | --- |
| Another computer on the network | The server listens on `127.0.0.1` only. There is no option to listen anywhere else. |
| A web page in another tab that sends a request to `127.0.0.1` | Every request under `/api/` needs a token: 128 random bits made when the server starts and put into the page it serves. It is never in the address. Another origin cannot read that page, so it cannot learn the token. The server sends no CORS headers, so the browser blocks the preflight of such a request before it is sent. |
| A forged write (CSRF) | A write must be a `POST` with the token in a header, a JSON content type, and an `Origin` equal to the server's own address. A form on another site can send none of the three. When the browser says where a request comes from (`Sec-Fetch-Site`), anything but the server's own page is refused. |
| DNS rebinding: a site that points its own name at `127.0.0.1` | The `Host` header must be `127.0.0.1:PORT` or `localhost:PORT`. Under any other name the server refuses everything, the page included. |
| Framing the page to trick a click (clickjacking) | `frame-ancestors 'none'` and `X-Frame-Options: DENY`. |
| Reading a response from another origin as a script or an image | `Cross-Origin-Resource-Policy: same-origin` and `X-Content-Type-Options: nosniff` on every response. |
| A hostile document | Raw HTML in the body and in notes is shown as text, never as elements. Diagrams are drawn with mermaid's strict setting. Images are not drawn. A Content-Security-Policy allows scripts, styles and requests from the server itself and nowhere else, so even injected markup could not run a script or send the document anywhere. |
| A hostile file name | The name reaches the page as JSON and is set as text. The line printed to the terminal has control characters replaced. |
| Reading other files | Nothing is read from disk by the name in a request. A path is looked up in the list of files built into the binary, and the only file read or written is the one named on the command line. |
| The page deciding who writes, or when | It cannot. The page sends an operation, such as "accept `s-4q8d`". The server adds the author it was started with, its own clock and a new id, performs the operation with the same code as the commands, and replaces the file in one step, or not at all if the file changed underneath. A request that names an author, a time or file contents is refused. |
| Oversized or malformed requests | Headers are limited to 16 KiB, a body to 1 MiB, and a request to 30 seconds. At most 32 connections are served at once. |

What it does **not** protect against:

- **Other users and other programs on the same computer.** Anything that can connect to `127.0.0.1` can fetch the page, read the token from it, read the file and write to it as the `--by` you gave. Do not run `endmark view` on a computer you share with people you do not trust. Use `--read-only` if you only want to look, though that still lets them read the file.
- **Browser extensions** that can read every page can read this one.
- **Whoever had the port before.** A page from another program that earlier ran on the same port is the same origin to the browser. If you ran something you do not trust on that port, pick another with `--port`.
- **The network.** There is no TLS, which is acceptable only because nothing leaves the computer. Do not forward the port or put a proxy in front of it. For a shared site, host the renderer yourself, behind real authentication.
- **A link in the document.** Following one leaves the page. The server sends `Referrer-Policy: no-referrer`, and links open without access to the page.
- `--read-only` protects the file from the page. It does not hide the file from anyone who could read it through the page.
