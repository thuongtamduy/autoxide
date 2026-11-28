// Checked by `tsc --noEmit` in CI: the types of the SDK follow docs/openapi.json.
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk_example", language: "vi" });

export async function example(): Promise<void> {
  // A public read: path parameter and query typed from the operation.
  const menu = await api.get("/api/v1/menus/public/{key}", { path: { key: "main" }, query: { locale: "vi" } });
  const labels: string[] = menu.items.map((item) => item.label);

  // A write with a typed body and a safe retry.
  const created = await api.post("/api/v1/menus", {
    body: { key: "footer", items: [{ label: "Contact", url: "mailto:hello@example.com" }] },
    idempotencyKey: crypto.randomUUID(),
  });
  const id: string = created.id;

  // Every item of a list, page after page.
  for await (const summary of api.paginate("/api/v1/menus", { query: { limit: 50 } })) {
    const key: string = summary.key;
    void key;
  }

  try {
    await api.delete("/api/v1/menus/{id}", { path: { id } });
  } catch (error) {
    if (error instanceof AutoxideError && error.code === "menus.menu_not_found") {
      return;
    }
    throw error;
  }
  void labels;
}

// `requestWithHeaders` + `ifMatch`, and the `refresh` option: added without changing the calls above.
let access = "";
const refreshing = createClient({
  baseUrl: "https://api.example.com",
  token: () => access,
  refresh: async () => {
    access = "next";
  },
});

export async function readThenChange(id: string): Promise<void> {
  const { data, etag, status, headers } = await refreshing.requestWithHeaders("get", "/api/v1/menus/{id}", { path: { id } });
  const key: string = data.key; // no `{ notModified: true }` here: no `ifNoneMatch` was passed
  const code: number = status;
  const requestId: string | null = headers.get("X-Request-Id");
  void [key, code, requestId];
  await refreshing.put("/api/v1/menus/{id}", { path: { id }, ifMatch: etag, body: { items: [] } });

  const again = await refreshing.requestWithHeaders("get", "/api/v1/menus/{id}", { path: { id }, ifNoneMatch: etag });
  if ("notModified" in again.data) return;
  const same: string = again.data.key;
  void same;
}
