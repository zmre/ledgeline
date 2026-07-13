// Pure helpers for AccountTreeSelect (WP-04): checkbox tri-state from the
// subtree-root selection set, the toggle that maintains the subtree-root
// invariant (shared by the filters and holdings stores), and search filtering
// that keeps ancestors visible. No Svelte/DOM imports — unit-tested under node.
import {accountMatches, type AccountNode} from "$lib/domain/accounts";

export type SelectionState = "checked" | "indeterminate" | "unchecked";

/**
 * Toggle an account in a subtree-root selection set, keeping the invariant
 * that the set stores ONLY subtree roots, never redundant children:
 * - name is selected → deselect it;
 * - name is covered by a selected ancestor → deselect that ancestor
 *   (drops the covering subtree; we cannot know its siblings here);
 * - otherwise → select it, pruning any now-redundant selected descendants.
 * Returns a NEW set; the input is never mutated.
 */
export function toggleSubtreeRoot(selected: ReadonlySet<string>, name: string): Set<string> {
    const accounts = new Set(selected);
    if (accounts.has(name)) {
        accounts.delete(name);
        return accounts;
    }
    const coveringAncestors = [...accounts].filter((sel) => accountMatches(sel, name));
    if (coveringAncestors.length > 0) {
        for (const sel of coveringAncestors) accounts.delete(sel);
        return accounts;
    }
    for (const sel of [...accounts]) {
        if (accountMatches(name, sel)) accounts.delete(sel);
    }
    accounts.add(name);
    return accounts;
}

/**
 * Tri-state for one tree node given the selected subtree roots:
 * checked when the node is selected or covered by a selected ancestor,
 * indeterminate when only some of its descendants are selected.
 */
export function selectionState(selected: ReadonlySet<string>, fullName: string): SelectionState {
    let hasSelectedDescendant = false;
    for (const sel of selected) {
        if (accountMatches(sel, fullName)) return "checked";
        if (accountMatches(fullName, sel)) hasSelectedDescendant = true;
    }
    return hasSelectedDescendant ? "indeterminate" : "unchecked";
}

/**
 * Filter the tree to nodes whose full name contains `query` (case-insensitive).
 * A matching node keeps its whole subtree; ancestors of matches stay visible.
 */
export function filterTree(nodes: AccountNode[], query: string): AccountNode[] {
    const q = query.trim().toLowerCase();
    if (q === "") return nodes;
    const walk = (list: AccountNode[]): AccountNode[] => {
        const out: AccountNode[] = [];
        for (const node of list) {
            if (node.fullName.toLowerCase().includes(q)) {
                out.push(node);
            } else {
                const children = walk(node.children);
                if (children.length > 0) out.push({...node, children});
            }
        }
        return out;
    };
    return walk(nodes);
}
