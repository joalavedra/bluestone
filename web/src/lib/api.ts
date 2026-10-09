import { QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query"

export const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 10_000, retry: (n, e) => !(e instanceof AuthError) && n < 2 } },
})

const TOKEN_KEY = "bluestone.token"

export class AuthError extends Error {}

export function getToken(): string | null {
  if (typeof window === "undefined") return null
  return window.localStorage.getItem(TOKEN_KEY)
}

export function setToken(token: string | null) {
  if (token) window.localStorage.setItem(TOKEN_KEY, token)
  else window.localStorage.removeItem(TOKEN_KEY)
  queryClient.clear()
  window.dispatchEvent(new Event("bluestone-auth"))
}

export async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api${path}`, {
    ...init,
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${getToken() ?? ""}`, ...init?.headers },
  })
  if (res.status === 401) throw new AuthError("Invalid or missing token")
  const body = await res.json().catch(() => ({}))
  if (!res.ok) throw new Error(body.error ?? `${res.status} ${res.statusText}`)
  return body as T
}

export type Status = "ok" | "low" | "out"

export interface ItemSummary {
  id: number
  brand: string
  sku: string
  name: string
  supplier: string | null
  tags: Array<string>
  channels: Array<string>
  on_hand: number
  sold_30d: number
  daily_velocity: number
  days_cover: number | null
  reorder_point: number | null
  target_stock: number | null
  lead_time_days: number
  suggested_reorder_qty: number
  unit_cost: number | null
  price: number | null
  image_url: string | null
  status: Status
}

export interface ChannelInfo {
  id: number
  kind: "shopify" | "prestashop" | "faire"
  name: string
  base_url: string
  last_synced_at: string | null
  last_sync_error: string | null
  listing_count: number
}

export interface BrandSummary {
  id: number
  name: string
  item_count: number
  low_stock_count: number
  out_of_stock_count: number
  stock_value: number
  channels: Array<ChannelInfo>
}

export interface ActivityEntry {
  id: number
  actor_kind: "agent" | "human" | "system"
  actor: string
  action: string
  subject_type: string | null
  subject_id: number | null
  summary: string
  created_at: string
}

export interface Proposal {
  id: number
  kind: "stock_adjustment" | "price_change" | "listing_status"
  status: "pending" | "applied" | "failed" | "rejected"
  item_id: number | null
  item_sku: string | null
  item_name: string | null
  brand: string | null
  listing_id: number
  channel: string
  before: Record<string, any>
  after: Record<string, any>
  warnings: Array<string>
  rationale: string | null
  actor: string
  decided_by: string | null
  decided_at: string | null
  error: string | null
  created_at: string
}

export interface DayPoint {
  day: string
  units: number
}

export interface Overview {
  brands: Array<BrandSummary>
  items: number
  low_stock: number
  out_of_stock: number
  pending_proposals: number
  units_30d: number
  revenue_30d: number
  stock_value: number
  attention: Array<ItemSummary>
  recent_activity: Array<ActivityEntry>
}

export interface Sales {
  period_days: number
  units: number
  revenue: number
  orders: number
  by_day: Array<DayPoint>
  top_items: Array<{ sku: string; name: string; brand: string; units: number; revenue: number }>
}

export interface ItemDetail extends ItemSummary {
  listings: Array<{
    id: number
    channel_id: number
    channel: string
    kind: string
    external_product_id: string
    external_variant_id: string
    sku: string | null
    title: string
    price: number | null
    status: string | null
    stock: Array<{ location: string; quantity: number }>
    synced_at: string
  }>
  notes: Array<{ id: number; body: string; actor: string; created_at: string }>
  sales_by_day: Array<DayPoint>
  proposals: Array<Proposal>
}

export interface Me {
  name: string
  kind: string
  scopes: Array<string>
}

const qs = (params: Record<string, string | number | undefined | null>) => {
  const s = new URLSearchParams()
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== null && v !== "") s.set(k, String(v))
  const out = s.toString()
  return out ? `?${out}` : ""
}

export interface ItemFilter {
  query?: string
  brand?: string
  tag?: string
  supplier?: string
  status?: string
}

export const useMe = () => useQuery({ queryKey: ["me"], queryFn: () => api<Me>("/me") })
export const useOverview = () => useQuery({ queryKey: ["overview"], queryFn: () => api<Overview>("/overview") })
export const useBrands = () => useQuery({ queryKey: ["brands"], queryFn: () => api<Array<BrandSummary>>("/brands") })
export const useItems = (f: ItemFilter) =>
  useQuery({ queryKey: ["items", f], queryFn: () => api<Array<ItemSummary>>(`/items${qs({ ...f })}`) })
export const useItem = (id: number) => useQuery({ queryKey: ["item", id], queryFn: () => api<ItemDetail>(`/items/${id}`) })
export const useTags = () => useQuery({ queryKey: ["tags"], queryFn: () => api<Array<{ name: string; items: number }>>("/tags") })
export const useSuppliers = () =>
  useQuery({
    queryKey: ["suppliers"],
    queryFn: () => api<Array<{ id: number; name: string; email: string | null; lead_time_days: number | null; items: number }>>("/suppliers"),
  })
export const useSales = (days: number, brand?: string) =>
  useQuery({ queryKey: ["sales", days, brand], queryFn: () => api<Sales>(`/sales${qs({ days, brand })}`) })
export const useProposals = (status: string) =>
  useQuery({ queryKey: ["proposals", status], queryFn: () => api<Array<Proposal>>(`/proposals${qs({ status })}`) })
export const useActivity = (actorKind: string) =>
  useQuery({
    queryKey: ["activity", actorKind],
    queryFn: () => api<Array<ActivityEntry>>(`/activity${qs({ actor_kind: actorKind, limit: 200 })}`),
  })

/** Mutation that refreshes every query on success — the dataset is small and views overlap heavily. */
export function useAction<TVars>(fn: (v: TVars) => Promise<unknown>) {
  const qc = useQueryClient()
  return useMutation({ mutationFn: fn, onSuccess: () => qc.invalidateQueries() })
}

export const post = (path: string, body?: unknown) => api(path, { method: "POST", body: JSON.stringify(body ?? {}) })
export const patch = (path: string, body: unknown) => api(path, { method: "PATCH", body: JSON.stringify(body) })
export const del = (path: string) => api(path, { method: "DELETE" })
