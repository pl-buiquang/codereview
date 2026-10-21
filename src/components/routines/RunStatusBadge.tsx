import { STATUS_LABEL } from "../../lib/routines";
import type { RoutineRunStatus } from "../../lib/types";

export function RunStatusBadge({ status }: { status: RoutineRunStatus }) {
  return <span className={`badge routine-status routine-status-${status}`}>{STATUS_LABEL[status]}</span>;
}
