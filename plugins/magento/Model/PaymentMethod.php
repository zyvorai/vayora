<?php
declare(strict_types=1);

namespace Zyvor\Payments\Model;

use Magento\Payment\Model\Method\AbstractMethod;
use Magento\Quote\Api\Data\CartInterface;

/**
 * Scaffold: redirects to Zyvor Checkout via a payment link.
 * Not production-ready — no Marketplace review, limited Magento version testing.
 */
class PaymentMethod extends AbstractMethod
{
    public const CODE = 'zyvor';

    protected $_code = self::CODE;
    protected $_isOffline = true;
    protected $_canAuthorize = false;
    protected $_canCapture = false;
    protected $_canUseInternal = false;
    protected $_canUseCheckout = true;

    public function isAvailable(?CartInterface $quote = null)
    {
        if (!parent::isAvailable($quote)) {
            return false;
        }
        $api = (string) $this->getConfigData('api_url');
        $key = (string) $this->getConfigData('api_key');
        $checkout = (string) $this->getConfigData('checkout_url');
        return $api !== '' && $key !== '' && $checkout !== '';
    }
}
