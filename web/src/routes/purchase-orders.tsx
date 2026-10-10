import { Link, createFileRoute } from "@tanstack/react-router"
import { Check, PackageCheck, Send, X } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { Empty, PageHeader } from "@/components/bits"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { post, useAction, usePurchaseOrders } from "@/lib/api"
import type { PoStatus, PurchaseOrder } from "@/lib/api"
import { ago, money, num } from "@/lib/format"
import { cn } from "@/lib/utils"

export const Route = createFileRoute("/purchase-orders")({ component: PurchaseOrdersPage })

const tone: Record<PoStatus, string> = {
  draft: "border-amber-200 bg-amber-50 text-amber-800",
  approved: "border-sky-200 bg-sky-50 text-sky-800",
  sent: "border-indigo-200 bg-indigo-50 text-indigo-800",
  partial: "border-violet-200 bg-violet-50 text-violet-800",
  received: "border-emerald-200 bg-emerald-50 text-emerald-800",
  cancelled: "border-zinc-200 bg-zinc-50 text-zinc-600",
}

function PurchaseOrdersPage() {
  const [tab, setTab] = useState("open")
  const { data: pos = [] } = usePurchaseOrders(tab)
  return (
    <>
      <PageHeader
        title="Purchase orders"
        description="Drafted by your agent or from Reorder. Approve, mark sent, then receive what arrives; in master mode receipts add stock and push it to every channel."
      />
      <Tabs value={tab} onValueChange={setTab} className="mb-4">
        <TabsList>
          <TabsTrigger value="open">Open</TabsTrigger>
          <TabsTrigger value="all">All</TabsTrigger>
        </TabsList>
      </Tabs>
      {!pos.length && (
        <Empty>
          No purchase orders. Draft one from{" "}
          <Link to="/reorder" className="underline">
            Reorder
          </Link>{" "}
          or ask your agent.
        </Empty>
      )}
      <div className="flex flex-col gap-4">
        {pos.map((po) => (
          <PoCard key={po.id} po={po} />
        ))}
      </div>
    </>
  )
}

function PoCard({ po }: { po: PurchaseOrder }) {
  const [qty, setQty] = useState<Record<number, string>>({})
  const act = useAction(({ path, body }: { path: string; body?: unknown }) =>
    post(`/purchase-orders/${po.id}/${path}`, body)
  )
  const run = (path: string, ok: string, body?: unknown) =>
    act.mutate(
      { path, body },
      {
        onSuccess: (r) => {
          const note = (r as { note?: string | null }).note
          toast.success(ok, note ? { description: note } : undefined)
          setQty({})
        },
        onError: (e) => toast.error(e.message),
      }
    )
  const receivable = po.status === "approved" || po.status === "sent" || po.status === "partial"
  const entered = po.lines
    .filter((l) => Number(qty[l.id]) > 0)
    .map((l) => ({ item: String(l.item_id), quantity: Number(qty[l.id]) }))
  return (
    <Card className="shadow-none">
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div>
          <CardTitle className="flex items-center gap-2">
            PO #{po.id} · {po.supplier ?? "No supplier"}
            <Badge variant="outline" className={cn("capitalize", tone[po.status])}>
              {po.status}
            </Badge>
          </CardTitle>
          <CardDescription>
            {po.brand}
            {po.warehouse ? ` → ${po.warehouse}` : ""} · {num(po.received_units)}/{num(po.units)} received ·{" "}
            {money(po.total_cost)} · drafted by {po.created_by} {ago(po.created_at)}
            {po.approved_by ? ` · approved by ${po.approved_by}` : ""}
            {po.note ? ` · ${po.note}` : ""}
          </CardDescription>
        </div>
        <div className="flex flex-wrap justify-end gap-2">
          {po.status === "draft" && (
            <Button size="sm" disabled={act.isPending} onClick={() => run("approve", `PO #${po.id} approved`)}>
              <Check className="size-4" /> Approve
            </Button>
          )}
          {po.status === "approved" && (
            <Button size="sm" variant="outline" disabled={act.isPending} onClick={() => run("send", `PO #${po.id} marked sent`)}>
              <Send className="size-4" /> Mark sent
            </Button>
          )}
          {receivable && (
            <Button
              size="sm"
              disabled={act.isPending}
              onClick={() =>
                run(
                  "receive",
                  entered.length ? "Received entered quantities" : `PO #${po.id} received in full`,
                  entered.length ? { lines: entered } : undefined
                )
              }
            >
              <PackageCheck className="size-4" /> {entered.length ? "Receive entered" : "Receive all"}
            </Button>
          )}
          {(po.status === "draft" || receivable) && (
            <Button
              size="sm"
              variant="ghost"
              disabled={act.isPending}
              onClick={() => run("cancel", `PO #${po.id} ${po.status === "partial" ? "closed" : "cancelled"}`)}
            >
              <X className="size-4" /> {po.status === "partial" ? "Close short" : "Cancel"}
            </Button>
          )}
        </div>
      </CardHeader>
      <CardContent>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Item</TableHead>
              <TableHead className="text-right">Ordered</TableHead>
              <TableHead className="text-right">Received</TableHead>
              <TableHead className="text-right">Unit cost</TableHead>
              <TableHead className="text-right">Line total</TableHead>
              {receivable && <TableHead className="w-32 text-right">Receive now</TableHead>}
            </TableRow>
          </TableHeader>
          <TableBody>
            {po.lines.map((l) => {
              const outstanding = l.quantity - l.received
              return (
                <TableRow key={l.id}>
                  <TableCell>
                    <Link to="/items/$itemId" params={{ itemId: String(l.item_id) }} className="font-medium hover:underline">
                      {l.name}
                    </Link>
                    <div className="text-muted-foreground text-xs">{l.sku}</div>
                  </TableCell>
                  <TableCell className="text-right tabular-nums">{num(l.quantity)}</TableCell>
                  <TableCell className={cn("text-right tabular-nums", outstanding === 0 && "text-emerald-700")}>
                    {num(l.received)}
                  </TableCell>
                  <TableCell className="text-right tabular-nums">{l.unit_cost === null ? "—" : money(l.unit_cost)}</TableCell>
                  <TableCell className="text-right tabular-nums">
                    {l.unit_cost === null ? "—" : money(l.unit_cost * l.quantity)}
                  </TableCell>
                  {receivable && (
                    <TableCell className="text-right">
                      {outstanding > 0 && (
                        <Input
                          type="number"
                          min={1}
                          max={outstanding}
                          placeholder={String(outstanding)}
                          value={qty[l.id] ?? ""}
                          onChange={(e) => setQty({ ...qty, [l.id]: e.target.value })}
                          className="ml-auto h-8 w-24 text-right"
                        />
                      )}
                    </TableCell>
                  )}
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  )
}
