import { Link, createFileRoute } from "@tanstack/react-router"
import { ArrowLeft, Plus, X } from "lucide-react"
import { useEffect, useState } from "react"
import { toast } from "sonner"
import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts"
import { ActorBadge, ChannelBadge, Empty, Kpi, StatusBadge, Thumb } from "@/components/bits"
import { ProposalCard } from "@/routes/approvals"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Textarea } from "@/components/ui/textarea"
import { del, patch, post, useAction, useItem } from "@/lib/api"
import type { ItemDetail } from "@/lib/api"
import { ago, money, num, shortDay } from "@/lib/format"

export const Route = createFileRoute("/items/$itemId")({ component: ItemPage })

function ItemPage() {
  const { itemId } = Route.useParams()
  const { data: item, error } = useItem(Number(itemId))
  if (error) return <Empty>{error.message}</Empty>
  if (!item) return null
  return (
    <>
      <Link to="/inventory" className="text-muted-foreground mb-3 inline-flex items-center gap-1 text-sm hover:text-slate-900">
        <ArrowLeft className="size-4" /> Inventory
      </Link>
      <div className="mb-6 flex flex-wrap items-center gap-4">
        <Thumb src={item.image_url} alt={item.name} />
        <div className="flex-1">
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-semibold tracking-tight">{item.name}</h1>
            <StatusBadge status={item.status} />
          </div>
          <div className="text-muted-foreground text-sm">
            {item.sku} · {item.brand}
          </div>
        </div>
        <ProposeDialog item={item} />
      </div>
      <div className="grid grid-cols-2 gap-3 md:grid-cols-5">
        <Kpi label="On hand" value={num(item.on_hand)} />
        <Kpi label="Forecast" value={`${num(item.daily_velocity, 2)}/d`} hint={`${item.forecast_method === "tsb" ? "intermittent (TSB)" : item.forecast_method === "ses" ? "smoothed (SES)" : "no sales"} · ${item.sold_30d} sold in 30d`} />
        <Kpi label="Days cover" value={item.days_cover === null ? "—" : `${num(item.days_cover, 1)}d`} tone={item.status === "ok" ? "default" : "warn"} />
        <Kpi label="Lead time" value={`${item.lead_time_days}d`} />
        <Kpi label="Suggested reorder" value={num(item.suggested_reorder_qty)} hint={`reorder at ${item.reorder_point ?? item.forecast_reorder_point} · safety ${item.safety_stock}`} tone={item.suggested_reorder_qty ? "accent" : "default"} />
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-3">
        <div className="flex flex-col gap-4 lg:col-span-2">
          <Card className="shadow-none">
            <CardHeader>
              <CardTitle>Listings</CardTitle>
              <CardDescription>Mirrored from each channel — the store stays the source of truth.</CardDescription>
            </CardHeader>
            <CardContent>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Channel</TableHead>
                    <TableHead>Title</TableHead>
                    <TableHead>Stock by location</TableHead>
                    <TableHead className="text-right">Price</TableHead>
                    <TableHead>Status</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {item.listings.map((l) => (
                    <TableRow key={l.id}>
                      <TableCell>
                        <ChannelBadge channel={`${l.kind}:${l.channel}`} />
                      </TableCell>
                      <TableCell className="max-w-56 truncate">{l.title}</TableCell>
                      <TableCell>
                        <div className="flex flex-col gap-0.5 text-sm">
                          {l.stock.map((s) => (
                            <span key={s.location}>
                              <span className="text-muted-foreground">{s.location}:</span> <span className="font-medium tabular-nums">{s.quantity}</span>
                            </span>
                          ))}
                        </div>
                      </TableCell>
                      <TableCell className="text-right tabular-nums">{money(l.price)}</TableCell>
                      <TableCell>
                        <Badge variant="outline" className="font-normal">
                          {l.status ?? "—"}
                        </Badge>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
          <Card className="shadow-none">
            <CardHeader>
              <CardTitle>Daily units sold</CardTitle>
              <CardDescription>Last 90 days</CardDescription>
            </CardHeader>
            <CardContent className="h-48">
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={item.sales_by_day} margin={{ left: -24, right: 4 }}>
                  <CartesianGrid vertical={false} stroke="var(--border)" />
                  <XAxis dataKey="day" tickFormatter={shortDay} tickLine={false} axisLine={false} fontSize={11} minTickGap={24} />
                  <YAxis tickLine={false} axisLine={false} fontSize={11} allowDecimals={false} />
                  <Tooltip labelFormatter={(d) => shortDay(String(d))} />
                  <Bar dataKey="units" fill="var(--chart-1)" radius={[3, 3, 0, 0]} />
                </BarChart>
              </ResponsiveContainer>
            </CardContent>
          </Card>
          <div>
            <h2 className="mb-2 font-semibold">Proposals</h2>
            <div className="flex flex-col gap-3">
              {item.proposals.map((p) => (
                <ProposalCard key={p.id} p={p} />
              ))}
              {!item.proposals.length && <Empty>No proposals for this item yet.</Empty>}
            </div>
          </div>
        </div>
        <div className="flex flex-col gap-4">
          <OrganiseCard item={item} />
          <TagsCard item={item} />
          <NotesCard item={item} />
        </div>
      </div>
    </>
  )
}

function OrganiseCard({ item }: { item: ItemDetail }) {
  const init = () => ({
    supplier: item.supplier ?? "",
    reorder_point: item.reorder_point?.toString() ?? "",
    target_stock: item.target_stock?.toString() ?? "",
    lead_time_days: item.lead_time_days.toString(),
    unit_cost: item.unit_cost?.toString() ?? "",
  })
  const [f, setF] = useState(init)
  useEffect(() => setF(init()), [item.id, item.supplier, item.reorder_point, item.target_stock, item.lead_time_days, item.unit_cost])
  const save = useAction(async () => {
    const n = (v: string) => (v.trim() === "" ? null : Number(v))
    if (f.supplier.trim() !== (item.supplier ?? "")) await post("/items/bulk/supplier", { item_ids: [item.id], supplier: f.supplier.trim() || null })
    const body = { reorder_point: n(f.reorder_point), target_stock: n(f.target_stock), lead_time_days: n(f.lead_time_days), unit_cost: n(f.unit_cost) }
    if (Object.values(body).some((v) => v !== null)) await patch(`/items/${item.id}`, body)
  })
  const field = (k: keyof typeof f, label: string, type = "number") => (
    <div className="flex flex-col gap-1.5">
      <Label htmlFor={k}>{label}</Label>
      <Input id={k} type={type} value={f[k]} onChange={(e) => setF({ ...f, [k]: e.target.value })} />
    </div>
  )
  return (
    <Card className="shadow-none">
      <CardHeader>
        <CardTitle>Organise</CardTitle>
        <CardDescription>Bluestone-only fields; your agent can set these too.</CardDescription>
      </CardHeader>
      <CardContent>
        <form
          className="grid grid-cols-2 gap-3"
          onSubmit={(e) => {
            e.preventDefault()
            save.mutate(undefined, { onSuccess: () => toast.success("Saved"), onError: (er) => toast.error(er.message) })
          }}
        >
          <div className="col-span-2">{field("supplier", "Supplier", "text")}</div>
          {field("reorder_point", "Reorder point")}
          {field("target_stock", "Target stock")}
          {field("lead_time_days", "Lead time (days)")}
          {field("unit_cost", "Unit cost (€)")}
          <Button type="submit" className="col-span-2" disabled={save.isPending}>
            Save
          </Button>
        </form>
      </CardContent>
    </Card>
  )
}

function TagsCard({ item }: { item: ItemDetail }) {
  const [tag, setTag] = useState("")
  const add = useAction((t: string) => post(`/items/${item.id}/tags`, { tags: [t] }))
  const remove = useAction((t: string) => del(`/items/${item.id}/tags/${encodeURIComponent(t)}`))
  return (
    <Card className="shadow-none">
      <CardHeader>
        <CardTitle>Tags</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="flex flex-wrap gap-1.5">
          {item.tags.map((t) => (
            <Badge key={t} variant="secondary" className="gap-1 pr-1 font-normal">
              {t}
              <button aria-label={`Remove ${t}`} onClick={() => remove.mutate(t)} className="hover:bg-primary/10 rounded-full p-0.5">
                <X className="size-3" />
              </button>
            </Badge>
          ))}
          {!item.tags.length && <span className="text-muted-foreground text-sm">No tags</span>}
        </div>
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            if (tag.trim()) add.mutate(tag.trim(), { onSuccess: () => setTag(""), onError: (er) => toast.error(er.message) })
          }}
        >
          <Input placeholder="Add tag" value={tag} onChange={(e) => setTag(e.target.value)} />
          <Button type="submit" variant="outline" size="icon" aria-label="Add tag">
            <Plus className="size-4" />
          </Button>
        </form>
      </CardContent>
    </Card>
  )
}

function NotesCard({ item }: { item: ItemDetail }) {
  const [body, setBody] = useState("")
  const add = useAction((b: string) => post(`/items/${item.id}/notes`, { body: b }))
  return (
    <Card className="shadow-none">
      <CardHeader>
        <CardTitle>Notes</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <form
          className="flex flex-col gap-2"
          onSubmit={(e) => {
            e.preventDefault()
            if (body.trim()) add.mutate(body.trim(), { onSuccess: () => setBody(""), onError: (er) => toast.error(er.message) })
          }}
        >
          <Textarea placeholder="Add a note for you and your agent" value={body} onChange={(e) => setBody(e.target.value)} />
          <Button type="submit" variant="outline" size="sm" className="self-end" disabled={!body.trim()}>
            Add note
          </Button>
        </form>
        {item.notes.map((n) => (
          <div key={n.id} className="rounded-lg border p-3">
            <div className="mb-1 flex items-center justify-between">
              <ActorBadge kind={n.actor.includes("agent") || n.actor.includes("claude") ? "agent" : "human"} name={n.actor} />
              <span className="text-muted-foreground text-xs">{ago(n.created_at)}</span>
            </div>
            <p className="text-sm whitespace-pre-wrap">{n.body}</p>
          </div>
        ))}
      </CardContent>
    </Card>
  )
}

function ProposeDialog({ item }: { item: ItemDetail }) {
  const [open, setOpen] = useState(false)
  const [kind, setKind] = useState("stock_adjustment")
  const [channel, setChannel] = useState(item.listings[0]?.channel ?? "")
  const listing = item.listings.find((l) => l.channel === channel)
  const [location, setLocation] = useState("")
  const [value, setValue] = useState("")
  const [rationale, setRationale] = useState("")
  const create = useAction((body: Record<string, unknown>) => post("/proposals", body))
  const loc = location || listing?.stock[0]?.location || ""
  const submit = () => {
    const base = { kind, item: String(item.id), channel, rationale: rationale || undefined }
    const body =
      kind === "stock_adjustment"
        ? { ...base, location: loc, quantity: Number(value) }
        : kind === "price_change"
          ? { ...base, price: Number(value) }
          : { ...base, status: value || "active" }
    create.mutate(body, {
      onSuccess: () => {
        toast.success("Proposal created — review it in Approvals")
        setOpen(false)
        setValue("")
        setRationale("")
      },
      onError: (e) => toast.error(e.message),
    })
  }
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="outline">Propose change</Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Propose a change</DialogTitle>
        </DialogHeader>
        <div className="grid grid-cols-2 gap-3">
          <div className="flex flex-col gap-1.5">
            <Label>Type</Label>
            <Select value={kind} onValueChange={(v) => { setKind(v); setValue("") }}>
              <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="stock_adjustment">Stock</SelectItem>
                <SelectItem value="price_change">Price</SelectItem>
                <SelectItem value="listing_status">Listing status</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>Channel</Label>
            <Select value={channel} onValueChange={setChannel}>
              <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
              <SelectContent>
                {item.listings.map((l) => (
                  <SelectItem key={l.id} value={l.channel}>{l.channel}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {kind === "stock_adjustment" && (
            <div className="col-span-2 flex flex-col gap-1.5">
              <Label>Location</Label>
              <Select value={loc} onValueChange={setLocation}>
                <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {listing?.stock.map((s) => (
                    <SelectItem key={s.location} value={s.location}>{s.location} (now {s.quantity})</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}
          <div className="col-span-2 flex flex-col gap-1.5">
            <Label htmlFor="pv">{kind === "stock_adjustment" ? "New quantity" : kind === "price_change" ? "New price (€)" : "Status"}</Label>
            {kind === "listing_status" ? (
              <Select value={value || "active"} onValueChange={setValue}>
                <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="active">active</SelectItem>
                  <SelectItem value="draft">draft</SelectItem>
                  <SelectItem value="archived">archived</SelectItem>
                </SelectContent>
              </Select>
            ) : (
              <Input id="pv" type="number" step="any" value={value} onChange={(e) => setValue(e.target.value)} />
            )}
          </div>
          <div className="col-span-2 flex flex-col gap-1.5">
            <Label htmlFor="pr">Rationale</Label>
            <Textarea id="pr" value={rationale} onChange={(e) => setRationale(e.target.value)} />
          </div>
        </div>
        <DialogFooter>
          <Button onClick={submit} disabled={create.isPending || (kind !== "listing_status" && !value)}>
            Create proposal
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
