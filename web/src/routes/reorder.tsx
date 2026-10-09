import { Link, createFileRoute } from "@tanstack/react-router"
import { Empty, PageHeader, StatusBadge } from "@/components/bits"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { useItems, useSuppliers } from "@/lib/api"
import type { ItemSummary } from "@/lib/api"
import { money, num } from "@/lib/format"

export const Route = createFileRoute("/reorder")({ component: ReorderPage })

function ReorderPage() {
  const { data: items = [] } = useItems({ status: "attention" })
  const { data: suppliers = [] } = useSuppliers()
  const groups = new Map<string, Array<ItemSummary>>()
  for (const i of items) {
    const k = i.supplier ?? "No supplier"
    groups.set(k, [...(groups.get(k) ?? []), i])
  }
  return (
    <>
      <PageHeader
        title="Reorder"
        description="Low and out-of-stock items grouped by supplier, with suggested quantities. Purchase-order drafting lands in phase 1."
      />
      {!items.length && <Empty>Nothing to reorder.</Empty>}
      <div className="flex flex-col gap-4">
        {[...groups.entries()].map(([name, list]) => {
          const s = suppliers.find((x) => x.name === name)
          const cost = list.reduce((acc, i) => acc + i.suggested_reorder_qty * (i.unit_cost ?? 0), 0)
          return (
            <Card key={name} className="shadow-none">
              <CardHeader>
                <CardTitle>{name}</CardTitle>
                <CardDescription>
                  {list.length} items
                  {s?.lead_time_days ? ` · ${s.lead_time_days}d lead time` : ""}
                  {s?.email ? ` · ${s.email}` : ""}
                  {cost > 0 ? ` · est. ${money(cost)} at cost` : ""}
                </CardDescription>
              </CardHeader>
              <CardContent>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Item</TableHead>
                      <TableHead>Brand</TableHead>
                      <TableHead className="text-right">On hand</TableHead>
                      <TableHead className="text-right">Velocity</TableHead>
                      <TableHead className="text-right">Days cover</TableHead>
                      <TableHead className="text-right">Lead time</TableHead>
                      <TableHead className="text-right">Suggested qty</TableHead>
                      <TableHead>Status</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {list.map((i) => (
                      <TableRow key={i.id}>
                        <TableCell>
                          <Link to="/items/$itemId" params={{ itemId: String(i.id) }} className="font-medium hover:underline">
                            {i.name}
                          </Link>
                          <div className="text-muted-foreground text-xs">{i.sku}</div>
                        </TableCell>
                        <TableCell>{i.brand}</TableCell>
                        <TableCell className="text-right tabular-nums">{num(i.on_hand)}</TableCell>
                        <TableCell className="text-right tabular-nums">{num(i.daily_velocity, 1)}/d</TableCell>
                        <TableCell className="text-right tabular-nums">{i.days_cover === null ? "—" : `${num(i.days_cover, 1)}d`}</TableCell>
                        <TableCell className="text-right tabular-nums">{i.lead_time_days}d</TableCell>
                        <TableCell className="text-primary text-right font-semibold tabular-nums">{num(i.suggested_reorder_qty)}</TableCell>
                        <TableCell>
                          <StatusBadge status={i.status} />
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          )
        })}
      </div>
    </>
  )
}
