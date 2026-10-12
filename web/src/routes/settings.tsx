import { createFileRoute } from "@tanstack/react-router"
import { CheckCircle2, Search, XCircle } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { PageHeader } from "@/components/bits"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { post, useAction, useBrands, useIntegrations } from "@/lib/api"
import type {
  IntegrationConnection,
  IntegrationConnector,
  IntegrationInput,
} from "@/lib/api"
import { ago } from "@/lib/format"

export const Route = createFileRoute("/settings")({ component: SettingsPage })

const categories = [
  ["all", "All"],
  ["sales_channel", "Sales channels"],
  ["accounting", "Accounting"],
  ["email", "Email"],
] as const

const sourceLabel: Record<IntegrationConnection["credential_source"], string> =
  {
    stored: "key stored encrypted",
    env: "env var",
    nango: "Nango",
    oauth: "provider login",
  }

interface Editing {
  connector: IntegrationConnector
  connection?: IntegrationConnection
}

function SettingsPage() {
  const { data } = useIntegrations()
  const [query, setQuery] = useState("")
  const [category, setCategory] = useState<string>("all")
  const [editing, setEditing] = useState<Editing | null>(null)
  const request = useAction((id: string) =>
    post("/integrations/request", { connector: id })
  )
  const q = query.trim().toLowerCase()
  const connectors = (data?.connectors ?? []).filter(
    (c) =>
      (category === "all" || c.category === category) &&
      (!q || `${c.name} ${c.id} ${c.description}`.toLowerCase().includes(q))
  )
  return (
    <>
      <PageHeader
        title="Settings"
        description="Integrations: find the shop, marketplace or accounting system a brand uses and connect it."
      />
      <div className="mb-4 flex flex-wrap items-center gap-3">
        <div className="relative w-full max-w-sm">
          <Search className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            className="pl-9"
            placeholder="Search connectors…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <Tabs value={category} onValueChange={(v) => setCategory(String(v))}>
          <TabsList>
            {categories.map(([id, label]) => (
              <TabsTrigger key={id} value={id}>
                {label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {connectors.map((c) => {
          const mine = (data?.connections ?? []).filter(
            (x) => x.connector === c.id
          )
          const soon = c.status === "coming_soon"
          return (
            <Card key={c.id} className="flex flex-col shadow-none">
              <CardHeader>
                <div className="flex items-start justify-between gap-2">
                  <CardTitle>{c.name}</CardTitle>
                  {soon ? (
                    <Badge variant="outline">Coming soon</Badge>
                  ) : mine.length ? (
                    <Badge>{mine.length} connected</Badge>
                  ) : (
                    <Badge variant="secondary">Available</Badge>
                  )}
                </div>
                <CardDescription>{c.description}</CardDescription>
              </CardHeader>
              <CardContent className="mt-auto flex flex-col gap-2">
                {mine.map((x) => (
                  <div
                    key={`${x.kind}-${x.id}`}
                    className="flex items-center justify-between gap-2 rounded-lg border bg-slate-50 px-3 py-2 text-sm"
                  >
                    <div className="min-w-0">
                      <div className="truncate font-medium">
                        {x.brand} · {x.label}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        {sourceLabel[x.credential_source]}
                        {x.kind === "channel" &&
                          ` · synced ${ago(x.last_synced_at)}`}
                      </div>
                      {x.error && (
                        <div
                          className="truncate text-xs text-red-600"
                          title={x.error}
                        >
                          {x.error}
                        </div>
                      )}
                    </div>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() =>
                        setEditing({ connector: c, connection: x })
                      }
                    >
                      Edit
                    </Button>
                  </div>
                ))}
                {soon ? (
                  <Button
                    variant="outline"
                    disabled={request.isPending}
                    onClick={() =>
                      request.mutate(c.id, {
                        onSuccess: () =>
                          toast.success(`Noted: you want ${c.name}`),
                        onError: (e) => toast.error(e.message),
                      })
                    }
                  >
                    Request {c.name}
                  </Button>
                ) : (
                  <Button
                    variant="outline"
                    onClick={() => setEditing({ connector: c })}
                  >
                    {mine.length ? "Add another" : "Set up"}
                  </Button>
                )}
              </CardContent>
            </Card>
          )
        })}
      </div>
      {!connectors.length && (
        <div className="py-10 text-center text-sm text-muted-foreground">
          No connector matches “{query}”. Missing one? Ask your agent or open an
          issue.
        </div>
      )}
      {editing && (
        <SetupDialog
          key={`${editing.connector.id}-${editing.connection?.id ?? "new"}`}
          {...editing}
          onClose={() => setEditing(null)}
        />
      )}
    </>
  )
}

function SetupDialog({
  connector,
  connection,
  onClose,
}: Editing & { onClose: () => void }) {
  const brands = useBrands()
  const [brand, setBrand] = useState(connection?.brand ?? "")
  const [fields, setFields] = useState<Record<string, string>>(
    connection?.fields ?? {}
  )
  const [secret, setSecret] = useState("")
  const [reference, setReference] = useState("")
  const [advanced, setAdvanced] = useState(
    connection?.credential_source === "env" ||
      connection?.credential_source === "nango"
  )
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(
    null
  )
  const cred = connector.credential
  const input = (): IntegrationInput => ({
    connector: connector.id,
    brand,
    fields,
    ...(advanced
      ? reference.trim() && { credential_ref: reference.trim() }
      : secret.trim() && { secret: secret.trim() }),
  })
  const test = useAction((i: IntegrationInput) => post("/integrations/test", i))
  const save = useAction(async (i: IntegrationInput) => {
    const saved = (await post("/integrations", i)) as {
      kind: string
      id: number
      needs_oauth: boolean
    }
    if (saved.needs_oauth) {
      const { url } = (await post(`/channels/${saved.id}/connect`)) as {
        url: string
      }
      window.location.href = url
    }
    return saved
  })
  const keepHint = connection
    ? `Leave empty to keep the saved credential (${sourceLabel[connection.credential_source]}).`
    : null
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {connection ? "Edit" : "Set up"} {connector.name}
          </DialogTitle>
          <DialogDescription>{connector.description}</DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault()
            save.mutate(input(), {
              onSuccess: (s) => {
                if ((s as { needs_oauth: boolean }).needs_oauth) return
                toast.success(`${connector.name} saved for ${brand}`)
                onClose()
              },
              onError: (err) => toast.error(err.message),
            })
          }}
        >
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="brand">Brand</Label>
            <Input
              id="brand"
              list="brands"
              required
              disabled={!!connection}
              placeholder="Northwind Coffee"
              value={brand}
              onChange={(e) => setBrand(e.target.value)}
            />
            <datalist id="brands">
              {(brands.data ?? []).map((b) => (
                <option key={b.id} value={b.name} />
              ))}
            </datalist>
          </div>
          {connector.fields.map((f) => (
            <div key={f.key} className="flex flex-col gap-1.5">
              <Label htmlFor={f.key}>
                {f.label}
                {!f.required && (
                  <span className="font-normal text-muted-foreground">
                    {" "}
                    (optional)
                  </span>
                )}
              </Label>
              <Input
                id={f.key}
                type={f.kind === "url" ? "url" : "text"}
                required={f.required}
                disabled={!!connection && f.key === "name"}
                placeholder={f.placeholder ?? f.default ?? ""}
                value={fields[f.key] ?? ""}
                onChange={(e) =>
                  setFields({ ...fields, [f.key]: e.target.value })
                }
              />
              {f.help && (
                <p className="text-xs text-muted-foreground">{f.help}</p>
              )}
            </div>
          ))}
          {cred && (
            <div className="flex flex-col gap-1.5 rounded-lg border p-3">
              {advanced ? (
                <>
                  <Label htmlFor="reference">Env var or Nango reference</Label>
                  <Input
                    id="reference"
                    placeholder={cred.reference_example}
                    value={reference}
                    onChange={(e) => setReference(e.target.value)}
                  />
                  <p className="text-xs text-muted-foreground">
                    For self-hosting: the key stays on your server, in an
                    environment variable or your own Nango (
                    <code>nango:&lt;integration&gt;/&lt;connection&gt;</code>
                    ). {keepHint}
                  </p>
                </>
              ) : (
                <>
                  <Label htmlFor="secret">{cred.label}</Label>
                  <Input
                    id="secret"
                    type="password"
                    autoComplete="off"
                    value={secret}
                    onChange={(e) => setSecret(e.target.value)}
                  />
                  <p className="text-xs text-muted-foreground">
                    {cred.help} Stored encrypted; never shown again. {keepHint}
                  </p>
                </>
              )}
              <button
                type="button"
                className="self-start text-xs font-medium text-primary hover:underline"
                onClick={() => setAdvanced(!advanced)}
              >
                {advanced
                  ? "Paste a key instead"
                  : "Advanced: use an env var or Nango"}
              </button>
            </div>
          )}
          {result && (
            <div
              className={
                result.ok
                  ? "flex items-start gap-2 rounded-lg bg-emerald-50 p-3 text-sm text-emerald-800"
                  : "flex items-start gap-2 rounded-lg bg-red-50 p-3 text-sm text-red-700"
              }
            >
              {result.ok ? (
                <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
              ) : (
                <XCircle className="mt-0.5 size-4 shrink-0" />
              )}
              <span className="break-words">{result.message}</span>
            </div>
          )}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={test.isPending || !brand.trim()}
              onClick={() =>
                test.mutate(input(), {
                  onSuccess: (r) =>
                    setResult(r as { ok: boolean; message: string }),
                  onError: (err) =>
                    setResult({ ok: false, message: err.message }),
                })
              }
            >
              {test.isPending ? "Testing…" : "Test connection"}
            </Button>
            <Button type="submit" disabled={save.isPending}>
              {cred?.oauth && !secret.trim() && !reference.trim() && !connection
                ? `Save and connect ${connector.name}`
                : "Save"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
