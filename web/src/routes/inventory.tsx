import { createFileRoute, useNavigate } from "@tanstack/react-router"
import { Search, Tag, Truck } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { ChannelBadge, Empty, PageHeader, StatusBadge, Thumb } from "@/components/bits"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { post, useAction, useBrands, useItems, useSuppliers, useTags } from "@/lib/api"
import { money, num } from "@/lib/format"

export const Route = createFileRoute("/inventory")({
  component: InventoryPage,
  validateSearch: (s: Record<string, unknown>): { status?: string; tag?: string } => ({
    status: typeof s.status === "string" ? s.status : undefined,
    tag: typeof s.tag === "string" ? s.tag : undefined,
  }),
})

const ALL = "__all"

function InventoryPage() {
  const search = Route.useSearch()
  const navigate = useNavigate({ from: "/inventory" })
  const [query, setQuery] = useState("")
  const [brand, setBrand] = useState(ALL)
  const [supplier, setSupplier] = useState(ALL)
  const status = search.status ?? "all"
  const tag = search.tag ?? ALL
  const pick = (v: string) => (v === ALL ? undefined : v)
  const { data: items = [], isLoading } = useItems({
    query,
    brand: pick(brand),
    tag: pick(tag),
    supplier: pick(supplier),
    status: status === "all" ? undefined : status,
  })
  const { data: brands = [] } = useBrands()
  const { data: tags = [] } = useTags()
  const { data: suppliers = [] } = useSuppliers()
  const [selected, setSelected] = useState<Set<number>>(new Set())
  const [dialog, setDialog] = useState<"tag" | "supplier" | null>(null)
  const [value, setValue] = useState("")
  const bulkTag = useAction((tagName: string) => post("/items/bulk/tags", { item_ids: [...selected], tags: [tagName] }))
  const bulkSupplier = useAction((name: string) => post("/items/bulk/supplier", { item_ids: [...selected], supplier: name }))
  const toggle = (id: number) =>
    setSelected((s) => {
      const n = new Set(s)
      if (n.has(id)) n.delete(id)
      else n.add(id)
      return n
    })
  const allSelected = items.length > 0 && items.every((i) => selected.has(i.id))

  const submit = () => {
    const m = dialog === "tag" ? bulkTag : bulkSupplier
    m.mutate(value.trim(), {
      onSuccess: () => {
        toast.success(`${dialog === "tag" ? "Tagged" : "Updated supplier for"} ${selected.size} items`)
        setDialog(null)
        setValue("")
        setSelected(new Set())
      },
      onError: (e) => toast.error(e.message),
    })
  }

  return (
    <>
      <PageHeader title="Inventory" description="Every item across brands and channels, with stock, velocity and cover." />
      <div className="mb-4 flex flex-wrap items-center gap-2">
        <Tabs value={status} onValueChange={(v) => navigate({ search: (s) => ({ ...s, status: v === "all" ? undefined : v }) })}>
          <TabsList>
            <TabsTrigger value="all">All</TabsTrigger>
            <TabsTrigger value="attention">Needs attention</TabsTrigger>
            <TabsTrigger value="low">Low</TabsTrigger>
            <TabsTrigger value="out">Out</TabsTrigger>
            <TabsTrigger value="ok">Healthy</TabsTrigger>
          </TabsList>
        </Tabs>
        <div className="relative ml-auto">
          <Search className="text-muted-foreground absolute top-2.5 left-2.5 size-4" />
          <Input className="w-64 bg-white pl-8" placeholder="Search SKU or name" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <Select value={brand} onValueChange={setBrand}>
          <SelectTrigger className="w-44 bg-white">
            <SelectValue placeholder="Brand" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>All brands</SelectItem>
            {brands.map((b) => (
              <SelectItem key={b.id} value={b.name}>
                {b.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={tag} onValueChange={(v) => navigate({ search: (s) => ({ ...s, tag: v === ALL ? undefined : v }) })}>
          <SelectTrigger className="w-36 bg-white">
            <SelectValue placeholder="Tag" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>All tags</SelectItem>
            {tags.map((t) => (
              <SelectItem key={t.name} value={t.name}>
                {t.name} ({t.items})
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={supplier} onValueChange={setSupplier}>
          <SelectTrigger className="w-40 bg-white">
            <SelectValue placeholder="Supplier" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>All suppliers</SelectItem>
            {suppliers.map((s) => (
              <SelectItem key={s.id} value={s.name}>
                {s.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {selected.size > 0 && (
        <div className="border-primary/30 bg-secondary mb-3 flex items-center gap-2 rounded-lg border px-4 py-2 text-sm">
          <span className="font-medium">{selected.size} selected</span>
          <Button size="sm" variant="outline" className="ml-auto bg-white" onClick={() => setDialog("tag")}>
            <Tag className="size-4" /> Add tag
          </Button>
          <Button size="sm" variant="outline" className="bg-white" onClick={() => setDialog("supplier")}>
            <Truck className="size-4" /> Set supplier
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())}>
            Clear
          </Button>
        </div>
      )}

      <Card className="py-0 shadow-none">
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow className="bg-slate-50/80">
                <TableHead className="w-10 pl-4">
                  <Checkbox
                    aria-label="Select all"
                    checked={allSelected}
                    onCheckedChange={() => setSelected(allSelected ? new Set() : new Set(items.map((i) => i.id)))}
                  />
                </TableHead>
                <TableHead>Item</TableHead>
                <TableHead>Channels</TableHead>
                <TableHead className="text-right">On hand</TableHead>
                <TableHead className="text-right">Sold 30d</TableHead>
                <TableHead className="text-right">Days cover</TableHead>
                <TableHead className="text-right">Reorder pt</TableHead>
                <TableHead className="text-right">Price</TableHead>
                <TableHead>Supplier</TableHead>
                <TableHead>Tags</TableHead>
                <TableHead>Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {items.map((i) => (
                <TableRow
                  key={i.id}
                  className="cursor-pointer"
                  data-state={selected.has(i.id) ? "selected" : undefined}
                  onClick={() => navigate({ to: "/items/$itemId", params: { itemId: String(i.id) } })}
                >
                  <TableCell className="pl-4" onClick={(e) => e.stopPropagation()}>
                    <Checkbox aria-label={`Select ${i.sku}`} checked={selected.has(i.id)} onCheckedChange={() => toggle(i.id)} />
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-3">
                      <Thumb src={i.image_url} alt={i.name} />
                      <div className="min-w-0">
                        <div className="max-w-64 truncate font-medium">{i.name}</div>
                        <div className="text-muted-foreground text-xs">
                          {i.sku} · {i.brand}
                        </div>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {i.channels.map((c) => (
                        <ChannelBadge key={c} channel={c} />
                      ))}
                    </div>
                  </TableCell>
                  <TableCell className="text-right font-medium tabular-nums">{num(i.on_hand)}</TableCell>
                  <TableCell className="text-right tabular-nums">{num(i.sold_30d)}</TableCell>
                  <TableCell className="text-right tabular-nums">{i.days_cover === null ? "—" : `${num(i.days_cover, 1)}d`}</TableCell>
                  <TableCell className="text-right tabular-nums">{num(i.reorder_point)}</TableCell>
                  <TableCell className="text-right tabular-nums">{money(i.price)}</TableCell>
                  <TableCell className="text-sm">{i.supplier ?? <span className="text-muted-foreground">—</span>}</TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {i.tags.map((t) => (
                        <Badge key={t} variant="secondary" className="font-normal">
                          {t}
                        </Badge>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell>
                    <StatusBadge status={i.status} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
          {!isLoading && !items.length && <Empty>No items match these filters.</Empty>}
        </CardContent>
      </Card>

      <Dialog open={dialog !== null} onOpenChange={(o) => !o && setDialog(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{dialog === "tag" ? `Tag ${selected.size} items` : `Set supplier for ${selected.size} items`}</DialogTitle>
          </DialogHeader>
          <form
            className="flex flex-col gap-2"
            onSubmit={(e) => {
              e.preventDefault()
              submit()
            }}
          >
            <Label htmlFor="bulk-value">{dialog === "tag" ? "Tag" : "Supplier name"}</Label>
            <Input
              id="bulk-value"
              autoFocus
              value={value}
              onChange={(e) => setValue(e.target.value)}
              list="bulk-options"
              placeholder={dialog === "tag" ? "e.g. bestseller" : "e.g. Café Imports"}
            />
            <datalist id="bulk-options">
              {(dialog === "tag" ? tags.map((t) => t.name) : suppliers.map((s) => s.name)).map((o) => (
                <option key={o} value={o} />
              ))}
            </datalist>
            <DialogFooter className="mt-2">
              <Button type="submit" disabled={!value.trim()}>
                Apply
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
