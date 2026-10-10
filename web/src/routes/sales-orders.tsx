import { Link, createFileRoute } from "@tanstack/react-router"
import { AlertTriangle, Ban, Check, PackageCheck, X } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { Empty, PageHeader } from "@/components/bits"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { post, useAction, useSalesOrders } from "@/lib/api"
import type { SalesOrder, SoStatus } from "@/lib/api"
import { ago, money, num } from "@/lib/format"
import { cn } from "@/lib/utils"

export const Route = createFileRoute("/sales-orders")({ component: SalesOrdersPage })

const tone: Record<SoStatus, string> = {
  proposed: "border-amber-200 bg-amber-50 text-amber-800",
  confirmed: "border-sky-200 bg-sky-50 text-sky-800",
  fulfilled: "border-emerald-200 bg-emerald-50 text-emerald-800",
  rejected: "border-zinc-200 bg-zinc-50 text-zinc-600",
  cancelled: "border-zinc-200 bg-zinc-50 text-zinc-600",
}

function SalesOrdersPage() {
  const [tab, setTab] = useState("open")
  const { data: orders = [] } = useSalesOrders(tab)
  return (
    <>
      <PageHeader
        title="Sales orders"
        description="Wholesale and off-platform orders your agent read from email, messages or photos. Confirm to reserve stock, fulfil when it ships."
      />
      <Tabs value={tab} onValueChange={setTab} className="mb-4">
        <TabsList>
          <TabsTrigger value="open">Open</TabsTrigger>
          <TabsTrigger value="all">All</TabsTrigger>
        </TabsList>
      </Tabs>
      {!orders.length && <Empty>No sales orders. Ask your agent to turn an order email or photo into one.</Empty>}
      <div className="flex flex-col gap-4">
        {orders.map((so) => (
          <SoCard key={so.id} so={so} />
        ))}
      </div>
    </>
  )
}

function SoCard({ so }: { so: SalesOrder }) {
  const act = useAction((path: string) => post(`/sales-orders/${so.id}/${path}`))
  const run = (path: string, ok: string) =>
    act.mutate(path, { onSuccess: () => toast.success(ok), onError: (e) => toast.error(e.message) })
  return (
    <Card className="shadow-none">
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div>
          <CardTitle className="flex items-center gap-2">
            SO #{so.id} · {so.customer}
            <Badge variant="outline" className={cn("capitalize", tone[so.status])}>
              {so.status}
            </Badge>
          </CardTitle>
          <CardDescription>
            {so.brand}
            {so.warehouse ? ` · ships from ${so.warehouse}` : ""}
            {so.external_ref ? ` · ref ${so.external_ref}` : ""} · {num(so.units)} units · {money(so.total)} · by{" "}
            {so.created_by} {ago(so.created_at)}
            {so.approved_by ? ` · confirmed by ${so.approved_by}` : ""}
          </CardDescription>
          {so.source && <div className="text-muted-foreground mt-1 text-xs">Source: {so.source}</div>}
          {so.note && <div className="text-muted-foreground text-xs">{so.note}</div>}
        </div>
        <div className="flex flex-col items-end gap-2">
          <div className="flex flex-wrap justify-end gap-2">
            {so.status === "proposed" && (
              <>
                <Button size="sm" variant="outline" disabled={act.isPending} onClick={() => run("reject", `SO #${so.id} rejected`)}>
                  <X className="size-4" /> Reject
                </Button>
                <Button size="sm" disabled={act.isPending} onClick={() => run("confirm", `SO #${so.id} confirmed, stock reserved`)}>
                  <Check className="size-4" /> Confirm
                </Button>
              </>
            )}
            {so.status === "confirmed" && (
              <>
                <Button size="sm" variant="ghost" disabled={act.isPending} onClick={() => run("cancel", `SO #${so.id} cancelled`)}>
                  <Ban className="size-4" /> Cancel
                </Button>
                <Button size="sm" disabled={act.isPending} onClick={() => run("fulfil", `SO #${so.id} fulfilled`)}>
                  <PackageCheck className="size-4" /> Fulfil
                </Button>
              </>
            )}
          </div>
          {so.warnings.map((w) => (
            <span key={w} className="inline-flex items-center gap-1 text-xs text-amber-700">
              <AlertTriangle className="size-3" /> {w}
            </span>
          ))}
        </div>
      </CardHeader>
      <CardContent>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Item</TableHead>
              <TableHead className="text-right">Qty</TableHead>
              <TableHead className="text-right">Available</TableHead>
              <TableHead className="text-right">Unit price</TableHead>
              <TableHead className="text-right">Line total</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {so.lines.map((l) => (
              <TableRow key={l.id}>
                <TableCell>
                  <Link to="/items/$itemId" params={{ itemId: String(l.item_id) }} className="font-medium hover:underline">
                    {l.name}
                  </Link>
                  <div className="text-muted-foreground text-xs">{l.sku}</div>
                </TableCell>
                <TableCell className="text-right tabular-nums">{num(l.quantity)}</TableCell>
                <TableCell
                  className={cn(
                    "text-right tabular-nums",
                    (so.status === "proposed" || so.status === "confirmed") && l.available < l.quantity && "text-red-600"
                  )}
                >
                  {num(l.available)}
                </TableCell>
                <TableCell className="text-right tabular-nums">{l.unit_price === null ? "—" : money(l.unit_price)}</TableCell>
                <TableCell className="text-right tabular-nums">
                  {l.unit_price === null ? "—" : money(l.unit_price * l.quantity)}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  )
}
