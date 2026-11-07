package dev.zyvor.vayora.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.LocalDate

class PayExtrasTest {
    @Test
    fun budgetsNearAndOver() {
        val b = Budgets(mapOf("food" to 100_00))
        val day = LocalDate.of(2026, 10, 10).toEpochDay()
        val entries = listOf(SpendEntry(50_00, SpendCategory.Food, day, true))
        val now = LocalDate.of(2026, 10, 20)
        assertNull(BudgetCheck.warning(10_00, SpendCategory.Food, entries, b, now))
        assertEquals(BudgetLevel.Over, BudgetCheck.warning(60_00, SpendCategory.Food, entries, b, now)?.level)
        assertNull(BudgetCheck.warning(999_00, SpendCategory.Travel, entries, b, now))
    }

    @Test
    fun circleLinkRoundTrip() {
        val s = CirclePayload.make("cir_abc123", "Asha K", Money(500_00), Money(2000_00))
        val l = CirclePayload.parse(s)
        assertEquals("cir_abc123", l.id)
        assertEquals("Asha K", l.primary)
        assertEquals(500_00L, l.perPayment.minor)
        var r = CircleRecord("cir_1", CircleRole.Delegate, "Asha", 500_00, 800_00)
        assertTrue(r.canCover(500_00))
        assertFalse(r.canCover(500_01))
        r = r.copy(status = "revoked")
        assertFalse(r.canCover(1))
    }

    @Test
    fun requestOpenAndFixed() {
        val open = RequestPayload.makeOpen("asha@okzyvor", "Asha K")
        val p = RequestPayload.parse(open)
        assertEquals("asha@okzyvor", p.payee)
        assertNull(p.amount)
        val fixed = RequestPayload.make("asha@okzyvor", "Asha", Money(10_00), "Tea")
        assertEquals(10_00L, RequestPayload.parse(fixed).amount?.minor)
    }

    @Test
    fun tipAndHourly() {
        assertEquals(50L, Tip.amount(1000, 5))
        assertEquals(0L, Tip.amount(0, 10))
        val day = LocalDate.of(2026, 10, 31)
        val zone = java.time.ZoneId.of("Asia/Kolkata")
        val nine = day.atTime(9, 0).atZone(zone).toInstant().toEpochMilli()
        val t = HourlyTotals.make(listOf(nine to 100L, nine to 50L), day, zone)
        assertEquals(150L, t[9])
        assertEquals("9a", HourlyTotals.label(9))
    }

    @Test
    fun voiceEnglishAndHindi() {
        val asha = Payee("asha@x", "Asha")
        val ravi = Payee("ravi@x", "Ravi Kumar")
        val ravi2 = Payee("ravi2@x", "Ravi Shah")
        val all = listOf(asha, ravi, ravi2)
        assertEquals(VoiceCommand.Result.Pay(asha, Money(200_00)), VoiceCommand.parse("Pay Asha 200 rupees", all))
        assertEquals(VoiceCommand.Result.Pay(asha, Money(450_00)), VoiceCommand.parse("Asha को ४५० रुपये भेजो", all))
        assertTrue(VoiceCommand.parse("pay Ravi 100", all) is VoiceCommand.Result.Ambiguous)
        assertEquals(VoiceCommand.Result.NoPayee("meena"), VoiceCommand.parse("pay Meena 100", all))
        assertEquals(VoiceCommand.Result.NotAPayment, VoiceCommand.parse("what is my balance", all))
        assertEquals(VoiceCommand.Result.NoPayee("asha"), VoiceCommand.parse("pay Asha 200", emptyList()))
    }
}
