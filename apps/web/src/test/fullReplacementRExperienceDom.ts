type DomTree = { node: string; value: string | null } | {
  tag: string;
  namespace: string | null;
  attributes: Record<string, string>;
  children: DomTree[];
};

export function snapshotDom(node: Node): DomTree {
  if (!(node instanceof Element)) return { node: node.nodeName, value: node.nodeValue };
  return {
    tag: node.localName,
    namespace: node.namespaceURI,
    attributes: Object.fromEntries(Array.from(node.attributes, attribute => [attribute.name, attribute.value])),
    children: Array.from(node.childNodes, snapshotDom),
  };
}
