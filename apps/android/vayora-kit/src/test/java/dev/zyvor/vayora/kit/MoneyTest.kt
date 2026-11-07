package dev.zyvor.vayora.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class MoneyTest {
    @Test
    fun parsesDecimalsAndFormatsIndianGrouping() {
        assertEquals(49900L, Money.parseDecimal("499")!!.minor)
        assertEquals(49950L, Money.parseDecimal("499.5")!!.minor)
        assertEquals(49950L, Money.parseDecimal("499.50")!!.minor)
        assertNull(Money.parseDecimal("0"))
        assertNull(Money.parseDecimal("499.999"))
        assertNull(Money.parseDecimal("-1"))
        assertEquals("₹1,23,456.00", Money(12_345_600).display)
        assertEquals("499.00", Money(49900).decimalString)
    }
}
