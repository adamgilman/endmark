// The JSON the core returns. See conformance/README.md, "Item keys".

export type ByteRange = [number, number];

export interface Quote {
  in: string | null;
  on: string;
}

export interface EditJson {
  in: string | null;
  replace: string;
  with: string;
  range: ByteRange | null;
}

export interface EventJson {
  verb: string;
  line: number;
  by: string;
  at: string;
  for: string | null;
  note: string;
}

export interface Item {
  id: string;
  kind: "comment" | "suggestion" | "edit" | "revert";
  line: number;
  by: string;
  at: string;
  for: string | null;
  note: string;
  re: string | null;
  state: string;
  events: EventJson[];
  // Comments.
  thread?: "open" | "resolved";
  root?: string;
  anchor?: "attached" | "detached" | "ambiguous" | "unquoted";
  quote?: Quote | null;
  range?: ByteRange | null;
  // Suggestions, edits and reverts.
  status?: "applicable" | "stale" | "located" | "unlocatable" | "reverted";
  reason?: string;
  reverted_by?: string | null;
  edits?: EditJson[];
}

export interface Diagnostic {
  line: number;
  code: string;
  id: string | null;
  message: string;
  hint: string;
  severity: string;
}

export interface DocumentJson {
  body: string;
  items: Item[];
  diagnostics: Diagnostic[];
  errors: number;
  warnings: number;
}

export interface Refusal {
  ok: false;
  code: string;
  message: string;
  hint: string;
}

export interface StatusOk {
  ok: true;
  document: DocumentJson;
}

export interface PreviewOk {
  ok: true;
  body: string;
}

export interface WriteOk {
  ok: true;
  source: string;
  document: DocumentJson;
  id: string;
  target: string | null;
  reanchored: string[];
  detached: string[];
  stale: string[];
}

export interface EditInput {
  in?: string;
  replace: string;
  with: string;
}

export interface Request {
  op: string;
  source: string;
  by?: string;
  at?: string;
  for?: string;
  note?: string;
  id?: string;
  new_id?: string;
  on?: string;
  in?: string;
  edits?: EditInput[];
}
