import { Link, createFileRoute } from "@tanstack/react-router"
import { AlertTriangle, ArrowRight, Check, X } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { ActorBadge, ChannelBadge, Empty, PageHeader } from "@/components/bits"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { post, useAction, useProposals } from "@/lib/api"
import type { Proposal } from "@/lib/api"
import { ago, money } from "@/lib/format"
import { cn } from "@/lib/utils"

export const Route = createFileRoute("/approvals")({ component: ApprovalsPage })

const kindLabel: Record<Proposal["kind"], string> = {
  stock_adjustment: "Stock adjustment",
  price_change: "Price change",
  listing_status: "Listing status",
  ledger_adjustment: "Stock (ledger)",
  stock_transfer: "Transfer",
}

const isStock = (kind: Proposal["kind"]) =>
  kind === "stock_adjustment" || kind === "ledger_adjustment"

function fmt(kind: Proposal["kind"], v: Record<string, any>) {
  if (isStock(kind)) return `${v.quantity} units`
  if (kind === "stock_transfer")
    return `${v.from} ${v.from_qty} · ${v.to} ${v.to_qty}`
  if (kind === "price_change") return money(v.price)
  return String(v.status ?? "—")
}

export function ProposalCard({ p }: { p: Proposal }) {
  const approve = useAction(() => post(`/proposals/${p.id}/approve`))
  const reject = useAction(() => post(`/proposals/${p.id}/reject`))
  const run = (m: typeof approve, verb: string) =>
    m.mutate(undefined, {
      onSuccess: (r: any) =>
        r?.status === "failed"
          ? toast.error(`#${p.id} failed: ${r.error}`)
          : toast.success(`Proposal #${p.id} ${verb}`),
      onError: (e) => toast.error(e.message),
    })
  const delta = isStock(p.kind)
    ? Number(p.after.quantity) - Number(p.before.quantity)
    : null
  return (
    <Card className="gap-0 py-0 shadow-none" data-testid={`proposal-${p.id}`}>
      <CardContent className="flex flex-wrap items-center gap-6 p-5">
        <div className="min-w-56 flex-1">
          <div className="flex items-center gap-2">
            <Badge variant="secondary">{kindLabel[p.kind]}</Badge>
            <ChannelBadge channel={p.channel} />
            <span className="text-xs text-muted-foreground">#{p.id}</span>
          </div>
          {p.item_id ? (
            <Link
              to="/items/$itemId"
              params={{ itemId: String(p.item_id) }}
              className="mt-2 block font-medium hover:underline"
            >
              {p.item_name}
            </Link>
          ) : null}
          <div className="text-xs text-muted-foreground">
            {p.item_sku} · {p.brand}
            {isStock(p.kind) && ` · ${p.after.location}`}
            {p.kind === "stock_transfer" && ` · move ${p.after.quantity}`}
          </div>
          {p.rationale && (
            <p className="mt-2 text-sm text-slate-700">“{p.rationale}”</p>
          )}
          <div className="mt-2 flex items-center gap-2 text-xs text-muted-foreground">
            <ActorBadge kind="agent" name={p.actor} /> proposed{" "}
            {ago(p.created_at)}
            {p.decided_by &&
              ` · ${p.status} by ${p.decided_by} ${ago(p.decided_at)}`}
          </div>
        </div>
        <div className="flex items-center gap-3 rounded-lg border bg-slate-50 px-4 py-3">
          <div className="text-center">
            <div className="text-[11px] text-muted-foreground uppercase">
              Before
            </div>
            <div className="font-semibold tabular-nums">
              {fmt(p.kind, p.before)}
            </div>
          </div>
          <ArrowRight className="size-4 text-muted-foreground" />
          <div className="text-center">
            <div className="text-[11px] text-muted-foreground uppercase">
              After
            </div>
            <div className="font-semibold text-primary tabular-nums">
              {fmt(p.kind, p.after)}
            </div>
          </div>
          {delta !== null && (
            <span
              className={cn(
                "text-sm font-medium",
                delta > 0 ? "text-emerald-600" : "text-red-600"
              )}
            >
              {delta > 0 ? `+${delta}` : delta}
            </span>
          )}
        </div>
        <div className="flex w-52 flex-col items-end gap-2">
          {p.warnings.map((w) => (
            <span
              key={w}
              className="inline-flex items-center gap-1 text-xs text-amber-700"
            >
              <AlertTriangle className="size-3" /> {w}
            </span>
          ))}
          {p.status === "pending" ? (
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={reject.isPending || approve.isPending}
                onClick={() => run(reject, "rejected")}
              >
                <X className="size-4" /> Reject
              </Button>
              <Button
                size="sm"
                disabled={approve.isPending || reject.isPending}
                onClick={() => run(approve, "applied")}
              >
                <Check className="size-4" />{" "}
                {approve.isPending ? "Applying…" : "Approve"}
              </Button>
            </div>
          ) : (
            <Badge
              variant="outline"
              className={cn(
                p.status === "applied" &&
                  "border-emerald-200 bg-emerald-50 text-emerald-700",
                p.status === "failed" && "border-red-200 bg-red-50 text-red-700"
              )}
            >
              {p.status}
            </Badge>
          )}
          {p.error && (
            <span className="text-right text-xs text-red-600">{p.error}</span>
          )}
        </div>
      </CardContent>
    </Card>
  )
}

function ApprovalsPage() {
  const [status, setStatus] = useState("pending")
  const { data = [] } = useProposals(status)
  return (
    <>
      <PageHeader
        title="Approvals"
        description="Changes your agents proposed. Nothing reaches Shopify, PrestaShop or the stock ledger until you approve it."
      />
      <Tabs value={status} onValueChange={setStatus} className="mb-4">
        <TabsList>
          <TabsTrigger value="pending">Pending</TabsTrigger>
          <TabsTrigger value="applied">Applied</TabsTrigger>
          <TabsTrigger value="failed">Failed</TabsTrigger>
          <TabsTrigger value="rejected">Rejected</TabsTrigger>
          <TabsTrigger value="all">All</TabsTrigger>
        </TabsList>
      </Tabs>
      <div className="flex flex-col gap-3">
        {data.map((p) => (
          <ProposalCard key={p.id} p={p} />
        ))}
        {!data.length && (
          <Empty>
            {status === "pending"
              ? "Inbox zero — no pending proposals."
              : `No ${status} proposals.`}
          </Empty>
        )}
      </div>
    </>
  )
}
