// Calendar tags retain their spelling so equal numeric versions keep input order.
export function calendarReleases(tags) {
  return Array.from(tags, (tag, order) => {
    const pieces = /^(\d{4})\.(0[1-9]|1[0-2])\.(\d+)$/.exec(tag);
    return pieces
      ? { tag, order, year: Number(pieces[1]), month: Number(pieces[2]), patch: Number(pieces[3]) }
      : null;
  })
    .filter(Boolean)
    .sort((a, b) => b.year - a.year || b.month - a.month || b.patch - a.patch || a.order - b.order);
}
