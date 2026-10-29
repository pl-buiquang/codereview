import { api } from "./api";

/**
 * Approve first, then enable auto-merge (squash): if the approval fails, nothing
 * is merged. Auto-merge is left alone when GitHub already has it on.
 */
export async function approveAndAutoMerge(
  owner: string,
  name: string,
  number: number,
  autoMergeOn: boolean,
): Promise<void> {
  await api.approvePr(owner, name, number);
  if (!autoMergeOn) await api.setPrAutoMerge(owner, name, number, true);
}
