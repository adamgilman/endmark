// Serves demo/dist on localhost. Run with `bun run serve` in packages/render.

import { join, normalize, resolve } from "node:path";

const dist = resolve(import.meta.dir, "../../demo/dist");

export function serve(port: number) {
  return Bun.serve({
    port,
    hostname: "127.0.0.1",
    async fetch(request) {
      const path = normalize(decodeURIComponent(new URL(request.url).pathname));
      const file = Bun.file(join(dist, path.endsWith("/") ? path + "index.html" : path));
      if (!path.startsWith("/") || path.includes("..") || !(await file.exists())) {
        return new Response("Not found", { status: 404 });
      }
      return new Response(file);
    },
  });
}

if (import.meta.main) {
  const server = serve(Number(process.env.PORT ?? 4173));
  console.log(`Serving ${dist} at http://${server.hostname}:${server.port}/`);
}
