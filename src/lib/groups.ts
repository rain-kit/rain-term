const KEY = "conn.groups";

/** 用户显式新建、还没有连接的分组。有连接的分组记在连接自己的 group 字段上。 */
export function loadGroups(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]") as unknown;
    if (!Array.isArray(v)) return [];
    return v.filter((x): x is string => typeof x === "string" && x.trim().length > 0);
  } catch {
    return [];
  }
}

export function saveGroups(groups: string[]) {
  localStorage.setItem(KEY, JSON.stringify(groups));
}
