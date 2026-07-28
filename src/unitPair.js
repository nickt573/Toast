// An amount and its unit ride together: both filled or both empty. This resolves the two
// inputs into what the backend stores, or an error the caller shows and stops on. Kept out
// of UnitPicker.jsx so that file exports only its component, which keeps Fast Refresh happy.
export function resolveUnitPair(value, variantId) {
  const hasValue = value !== "" && value !== null && value !== undefined && !Number.isNaN(parseFloat(value));
  const num = hasValue ? parseFloat(value) : null;
  const hasUnit = variantId !== null && variantId !== undefined;
  if (hasValue && num <= 0) return { error: "A unit amount has to be more than 0." };
  if (hasValue && !hasUnit) return { error: "Pick a unit for the amount you entered." };
  if (hasUnit && !hasValue) return { error: "Enter an amount to go with the unit." };
  return { numValue: num, variantId: hasUnit ? variantId : null };
}
