package dev.zyvor.vayora.kit

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId

enum class SpendCategory(val raw: String, val title: String) {
    Food("food", "Food"),
    Shopping("shopping", "Shopping"),
    Bills("bills", "Bills"),
    Travel("travel", "Travel"),
    Transfers("transfers", "Transfers"),
    Other("other", "Other");

    companion object {
        val editable: List<SpendCategory> = listOf(Food, Shopping, Bills, Travel, Other)

        fun guess(name: String, note: String, transfer: Boolean): SpendCategory {
            if (transfer) return Transfers
            val t = "$name $note".lowercase()
            val table = listOf(
                Food to listOf("cafe", "chai", "tea", "coffee", "restaurant", "food", "pizza", "bakery", "dhaba", "swiggy", "zomato"),
                Bills to listOf("electric", "water", "gas", "broadband", "recharge", "rent", "bill", "insurance", "dth"),
                Travel to listOf("cab", "uber", "ola", "metro", "bus", "train", "fuel", "petrol", "flight", "toll"),
                Shopping to listOf("store", "mart", "shop", "mall", "market", "stores", "fashion"),
            )
            for ((c, words) in table) if (words.any { t.contains(it) }) return c
            return Other
        }
    }
}

data class SpendEntry(
    val amountMinor: Long,
    val category: SpendCategory,
    val epochDay: Long,
    val paid: Boolean,
)

data class Budgets(val limits: Map<String, Long> = emptyMap()) {
    fun limit(c: SpendCategory): Long? = limits[c.raw]?.takeIf { it > 0 }

    fun set(c: SpendCategory, minor: Long?): Budgets {
        val next = limits.toMutableMap()
        if (minor != null && minor > 0) next[c.raw] = minor else next.remove(c.raw)
        return Budgets(next)
    }
}

enum class BudgetLevel { Ok, Near, Over }

data class BudgetStatus(
    val category: SpendCategory,
    val spentMinor: Long,
    val limitMinor: Long,
) {
    val fraction: Double get() = if (limitMinor == 0L) 0.0 else spentMinor.toDouble() / limitMinor.toDouble()
    val level: BudgetLevel
        get() = when {
            spentMinor > limitMinor -> BudgetLevel.Over
            spentMinor.toDouble() >= 0.8 * limitMinor -> BudgetLevel.Near
            else -> BudgetLevel.Ok
        }
    val leftMinor: Long get() = maxOf(0, limitMinor - spentMinor)

    fun warningText(): String = when (level) {
        BudgetLevel.Over -> "This puts ${category.title} ${Money(spentMinor - limitMinor).display} over your budget"
        BudgetLevel.Near -> "${category.title} will be at ${(fraction * 100).toInt()}% of your budget"
        BudgetLevel.Ok -> ""
    }
}

object MonthSummary {
    fun byCategory(entries: List<SpendEntry>, now: LocalDate = LocalDate.now()): List<Pair<SpendCategory, Long>> {
        val sums = mutableMapOf<SpendCategory, Long>()
        for (e in entries) {
            if (!e.paid) continue
            val d = LocalDate.ofEpochDay(e.epochDay)
            if (d.year != now.year || d.month != now.month) continue
            sums[e.category] = (sums[e.category] ?: 0) + e.amountMinor
        }
        return sums.entries
            .map { it.key to it.value }
            .sortedWith(compareByDescending<Pair<SpendCategory, Long>> { it.second }.thenBy { it.first.raw })
    }
}

object BudgetCheck {
    fun statuses(entries: List<SpendEntry>, budgets: Budgets, now: LocalDate = LocalDate.now()): List<BudgetStatus> {
        val spent = MonthSummary.byCategory(entries, now).toMap()
        return SpendCategory.entries.mapNotNull { c ->
            val l = budgets.limit(c) ?: return@mapNotNull null
            BudgetStatus(c, spent[c] ?: 0, l)
        }.sortedWith(compareByDescending<BudgetStatus> { it.fraction }.thenBy { it.category.raw })
    }

    fun warning(
        adding: Long,
        to: SpendCategory,
        entries: List<SpendEntry>,
        budgets: Budgets,
        now: LocalDate = LocalDate.now(),
    ): BudgetStatus? {
        val s = statuses(entries, budgets, now).firstOrNull { it.category == to } ?: return null
        val after = BudgetStatus(to, s.spentMinor + adding, s.limitMinor)
        return if (after.level == BudgetLevel.Ok) null else after
    }
}

fun LocalTxn.toSpendEntry(category: SpendCategory, whenMillis: Long = System.currentTimeMillis()): SpendEntry =
    SpendEntry(
        amountMinor = amount.minor,
        category = category,
        epochDay = Instant.ofEpochMilli(whenMillis).atZone(ZoneId.systemDefault()).toLocalDate().toEpochDay(),
        paid = ok,
    )
