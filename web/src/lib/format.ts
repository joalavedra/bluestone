export const num = (n: number | null | undefined, digits = 0) =>
  n === null || n === undefined ? "—" : n.toLocaleString("en-US", { maximumFractionDigits: digits, minimumFractionDigits: 0 })

export const money = (n: number | null | undefined) =>
  n === null || n === undefined ? "—" : n.toLocaleString("en-US", { style: "currency", currency: "EUR", maximumFractionDigits: 2 })

export const compactMoney = (n: number) =>
  n.toLocaleString("en-US", { style: "currency", currency: "EUR", notation: "compact", maximumFractionDigits: 1 })

export function ago(iso: string | null | undefined) {
  if (!iso) return "never"
  const s = Math.round((Date.now() - new Date(iso).getTime()) / 1000)
  if (s < 60) return "just now"
  if (s < 3600) return `${Math.floor(s / 60)}m ago`
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`
  return `${Math.floor(s / 86400)}d ago`
}

export const shortDay = (d: string) => new Date(`${d}T00:00:00Z`).toLocaleDateString("en-US", { month: "short", day: "numeric" })
