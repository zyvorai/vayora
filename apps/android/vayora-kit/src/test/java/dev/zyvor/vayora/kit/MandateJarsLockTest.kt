package dev.zyvor.vayora.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MandateJarsLockTest {
    @Test
    fun mandateParse() {
        val p = MandatePayload.parse(
            "upi://mandate?pa=shop@upi&pn=Shop&am=500.00&cu=INR&tn=md_abc1&recur=monthly&start=2026-01-01&end=2027-01-01",
        )
        assertEquals("shop@upi", p.payee)
        assertEquals(Money(50_000), p.cap)
        assertEquals(MandateFrequency.Monthly, p.frequency)
    }

    @Test
    fun jarsConserveMoney() {
        val book = JarBook()
        val jar = book.add("Rent", 10_000_00)
        var wallet = 50_000_00L
        val (w2, _) = book.deposit(jar.id, 2_000_00, wallet)
        wallet = w2
        assertEquals(48_000_00L, wallet)
        assertEquals(2_000_00L, book.jars.first().savedMinor)
        wallet = book.remove(jar.id, wallet)
        assertEquals(50_000_00L, wallet)
        assertTrue(book.jars.isEmpty())
    }

    @Test
    fun lockPolicy() {
        assertTrue(LockPolicy.shouldLock(true, null, 1_000L, 5))
        assertFalse(LockPolicy.shouldLockOnReturn(true, null, 1_000L, 5))
        assertTrue(LockPolicy.shouldLockOnReturn(true, 0L, 6 * 60_000L, 5))
        assertTrue(ScamHints.check("Prize claim", "") != null)
        assertTrue(ScamHints.check("Tea stall", "lunch") == null)
    }
}
