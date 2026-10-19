import { FileCheck, FileUp } from "lucide-react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { post, useAccounting, useAction, useErp } from "@/lib/api"
import type { AccountingDocument } from "@/lib/api"

const systems: Record<string, string> = {
  quickbooks: "QuickBooks",
  holded: "Holded",
}

/** Sends a PO (as a bill) or SO (as an invoice) to the brand's accounting system, or shows the result. */
export function BooksButton({
  brand,
  source,
  id,
  ready,
}: {
  brand: string
  source: AccountingDocument["source"]
  id: number
  ready: boolean
}) {
  const { data } = useAccounting()
  const act = useAction(() =>
    post(
      `/${source === "purchase_order" ? "purchase-orders" : "sales-orders"}/${id}/export`
    )
  )
  const conn = data?.connections.find((c) => c.brand === brand)
  if (!conn) return null
  const system = systems[conn.kind] ?? conn.kind
  const what = source === "purchase_order" ? "bill" : "invoice"
  const doc = data?.documents.find(
    (d) =>
      d.source === source && d.source_id === id && d.connection_id === conn.id
  )
  if (doc?.status === "exported")
    return (
      <Badge
        variant="outline"
        className="h-8 border-emerald-200 bg-emerald-50 text-emerald-800"
      >
        <FileCheck className="size-3.5" /> {system} {what} #{doc.external_id}
      </Badge>
    )
  if (!ready) return null
  return (
    <Button
      size="sm"
      variant="outline"
      disabled={act.isPending}
      title={doc?.status === "failed" ? (doc.error ?? undefined) : undefined}
      onClick={() =>
        act.mutate(undefined, {
          onSuccess: () =>
            toast.success(
              `${what === "bill" ? "Bill" : "Invoice"} created in ${system}`
            ),
          onError: (e) => toast.error(e.message),
        })
      }
    >
      <FileUp className="size-4" />
      {doc?.status === "failed"
        ? `Retry ${what}`
        : `${what === "bill" ? "Bill" : "Invoice"} in ${system}`}
    </Button>
  )
}

const erps: Record<string, string> = { holded: "Holded" }

/** Sends a confirmed SO to the brand's ERP as an order, or shows the result. */
export function ErpButton({
  brand,
  id,
  ready,
}: {
  brand: string
  id: number
  ready: boolean
}) {
  const { data } = useErp()
  const act = useAction(() => post(`/sales-orders/${id}/erp`))
  const conn = data?.connections.find((c) => c.brand === brand)
  if (!conn) return null
  const system = erps[conn.kind] ?? conn.kind
  const order = data?.orders.find(
    (o) => o.sales_order_id === id && o.connection_id === conn.id
  )
  if (order?.status === "exported")
    return (
      <Badge
        variant="outline"
        className="h-8 border-emerald-200 bg-emerald-50 text-emerald-800"
      >
        <FileCheck className="size-3.5" /> {system} order #{order.external_id}
      </Badge>
    )
  if (!ready) return null
  return (
    <Button
      size="sm"
      variant="outline"
      disabled={act.isPending}
      title={
        order?.status === "failed" ? (order.error ?? undefined) : undefined
      }
      onClick={() =>
        act.mutate(undefined, {
          onSuccess: () => toast.success(`Order created in ${system}`),
          onError: (e) => toast.error(e.message),
        })
      }
    >
      <FileUp className="size-4" />
      {order?.status === "failed" ? "Retry ERP order" : `Send to ${system}`}
    </Button>
  )
}
