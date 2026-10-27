import { createContext, useContext, type ReactNode } from "react";
import { useHistoryEntries } from "./useHistoryEntries";
import { useDictationMetrics } from "./useDictationMetrics";

type DictationData = {
  history: ReturnType<typeof useHistoryEntries>;
  timing: ReturnType<typeof useDictationMetrics>;
};
const DataContext = createContext<DictationData | null>(null);

export const DictationDataProvider = ({
  children,
}: {
  children: ReactNode;
}) => {
  const history = useHistoryEntries();
  const timing = useDictationMetrics();
  return (
    <DataContext.Provider value={{ history, timing }}>
      {children}
    </DataContext.Provider>
  );
};

export function useDictationData(): DictationData {
  const data = useContext(DataContext);
  if (!data) throw new Error("DictationDataProvider is required");
  return data;
}
